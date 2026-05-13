/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use anyhow::Error;
use log::{error, info, warn};
use tokio::sync::broadcast;
use wtransport::{Endpoint, Identity, ServerConfig};

use crate::appcontext::AppContext;
use crate::streamdef::DataFrame;

/// Generate a 14-day self-signed identity for the QUIC endpoint and return its
/// SHA-256 fingerprint in dotted-hex format ("aa:bb:cc:…").
///
/// Browsers require ≤14 days validity when using `serverCertificateHashes`.
/// The SANs include common local names; the hash-based check bypasses SAN
/// validation in Chrome, so non-localhost origins work too.
pub fn build_identity(extra_sans: &[String]) -> Result<(Identity, Vec<u8>), Error> {
    let mut sans: Vec<&str> = vec!["localhost", "127.0.0.1", "::1"];
    for s in extra_sans {
        sans.push(s.as_str());
    }

    let identity = Identity::self_signed_builder()
        .subject_alt_names(sans)
        .from_now_utc()
        .validity_days(14)
        .build()?;

    let fingerprint = identity.certificate_chain().as_slice()[0]
        .hash()
        .as_ref()
        .to_vec();

    Ok((identity, fingerprint))
}

/// Wire-frame format sent over the unidirectional stream:
///   [4 bytes LE: json_len][json_len bytes: UTF-8 JSON]
///   [4 bytes LE: data_len][data_len bytes: binary]
///   … repeated for every frame
async fn pump_frames(
    mut stream: wtransport::stream::SendStream,
    mut rx: broadcast::Receiver<DataFrame>,
    connection: &wtransport::Connection,
) -> Result<(), Error> {
    loop {
        tokio::select! {
            biased;
            result = rx.recv() => {
                match result {
                    Ok(frame) => {
                        let json_bytes = serde_json::to_vec(&frame.metadata)?;
                        let json_len = json_bytes.len() as u32;
                        let data_len = frame.data.len() as u32;

                        stream.write_all(&json_len.to_le_bytes()).await?;
                        stream.write_all(&json_bytes).await?;
                        stream.write_all(&data_len.to_le_bytes()).await?;
                        stream.write_all(&frame.data).await?;
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        warn!("WebTransport receiver lagged {n} frames, skipping");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            _ = connection.closed() => break,
        }
    }
    Ok(())
}

pub async fn run(app_context: AppContext, identity: Identity, port: u16) -> Result<(), Error> {
    let config = ServerConfig::builder()
        .with_bind_default(port)
        .with_identity(identity)
        .build();

    let endpoint = Endpoint::server(config)?;
    info!("WebTransport (QUIC) server listening on UDP port {port}");

    loop {
        let incoming = endpoint.accept().await;
        let app_context = app_context.clone();

        tokio::spawn(async move {
            let session_request = match incoming.await {
                Ok(sr) => sr,
                Err(e) => {
                    warn!("WebTransport incoming connection error: {e}");
                    return;
                }
            };

            let path = percent_encoding::percent_decode_str(session_request.path())
                .decode_utf8_lossy()
                .into_owned();
            let remote = session_request.remote_address();
            info!("WebTransport session request from {remote} for path {path}");

            let Some(stream_def) = app_context.streams.get(&path) else {
                warn!("Unknown WebTransport path: {path}");
                session_request.not_found().await;
                return;
            };
            let stream_def = stream_def.clone();

            let connection = match session_request.accept().await {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to accept WebTransport session on {path}: {e}");
                    return;
                }
            };
            info!("WebTransport session accepted for {path} from {remote}");

            let rx = {
                let mut ctx = stream_def.lock().unwrap();
                let rx = ctx.tx.subscribe();
                ctx.count += 1;

                let should_start = ctx.count == 1
                    || ctx.task.as_ref().map(|t| t.is_finished()).unwrap_or(true);
                if should_start {
                    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel();
                    let url = ctx.url.clone();
                    let transport = ctx.transport.clone();
                    let tx = ctx.tx.clone();
                    let path_log = path.clone();
                    ctx.stop_tx = Some(stop_tx);
                    ctx.task = Some(tokio::spawn(async move {
                        info!("RTSP {path_log} started (WebTransport)");
                        if let Err(e) =
                            crate::rtspclient::run_until(url, transport, tx, stop_rx).await
                        {
                            error!("RTSP {path_log} exited with error: {e}");
                        }
                        info!("RTSP {path_log} stopped (WebTransport)");
                    }));
                }
                rx
            };

            let result = async {
                let opening = connection.open_uni().await?;
                let send_stream = opening.await?;
                pump_frames(send_stream, rx, &connection).await
            }
            .await;

            if let Err(e) = result {
                warn!("WebTransport session error on {path}: {e}");
            }

            let mut ctx = stream_def.lock().unwrap();
            if ctx.count > 0 {
                ctx.count -= 1;
            }
            if ctx.count == 0 {
                if let Some(stop_tx) = ctx.stop_tx.take() {
                    let _ = stop_tx.send(());
                }
                ctx.task.take();
            }
        });
    }
}
