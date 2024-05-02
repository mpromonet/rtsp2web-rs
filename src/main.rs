/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use anyhow::{anyhow, Error};
use clap::Parser;
use futures::{StreamExt, SinkExt};
use log::{error, info, debug};
use retina::client::{SessionGroup, SetupOptions};
use retina::codec::CodecItem;
use std::sync::Arc;
use tokio::sync::broadcast;

use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::protocol::Message;

#[derive(Parser)]
pub struct Opts {
    /// `rtsp://` URL to connect to.
    #[clap(long)]
    url: url::Url,
}

pub async fn run(opts: Opts, tx: broadcast::Sender<Vec<u8>>) -> Result<(), Error> {
    let session_group = Arc::new(SessionGroup::default());
    let r = run_inner(opts, session_group.clone(), tx).await;
    if let Err(e) = session_group.await_teardown().await {
        error!("TEARDOWN failed: {}", e);
    }
    r
}

async fn run_inner(opts: Opts, session_group: Arc<SessionGroup>, tx: broadcast::Sender<Vec<u8>>) -> Result<(), Error> {
    let stop = tokio::signal::ctrl_c();

    let mut session = retina::client::Session::describe(
        opts.url,
        retina::client::SessionOptions::default()
            .session_group(session_group),
    )
    .await?;
    info!("{:?}", session.streams());

    let video_stream = session
        .streams()
        .iter()
        .position(|s| {
            matches!(
                s.parameters(),
                Some(retina::codec::ParametersRef::Video(..))
            )
        })
        .ok_or_else(|| anyhow!("couldn't find video stream"))?;

    session
        .setup(video_stream, SetupOptions::default())
        .await?;

    let mut videosession = session
        .play(retina::client::PlayOptions::default())
        .await?
        .demuxed()?;

    tokio::pin!(stop);
    loop {
        tokio::select! {
            item = videosession.next() => {
                match item.ok_or_else(|| anyhow!("EOF"))?? {
                    CodecItem::VideoFrame(m) => {
                        debug!(
                            "{}: size:{}\n",
                            m.timestamp().timestamp(),
                            m.data().len(),
                        );
                        if let Err(e) = tx.send(m.data().to_vec()) {
                            error!("Error broadcasting message: {}", e);
                        }                        
                    },
                    _ => continue,
                };
            },
            _ = &mut stop => {
                break;
            },
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() {
    let _ = env_logger::init();

    let (tx, rx) = broadcast::channel::<Vec<u8>>(100);
    // Start the WebSocket server
    info!("start websocket");
    let listener = TcpListener::bind("0.0.0.0:9001").await.unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let receiver = rx.resubscribe();
            tokio::spawn(accept_connection(stream, receiver));
        }
    });

    // Start the RTSP client
    info!("start rtsp client");
    if let Err(e) = {
        let opts = Opts::parse();
        run(opts, tx).await
    } {
        error!("Fatal: {}", itertools::join(e.chain(), "\ncaused by: "));
        std::process::exit(1);
    }


    info!("Done");
}

async fn accept_connection(stream: tokio::net::TcpStream, mut rx: broadcast::Receiver<Vec<u8>>) {
    let mut websocket = accept_async(stream).await.unwrap();

    while let Ok(msg) = rx.recv().await {
        match websocket.send(Message::Binary(msg.to_vec())).await {
            Ok(_) => {},
            Err(e) => match e {
                tokio_tungstenite::tungstenite::Error::ConnectionClosed | tokio_tungstenite::tungstenite::Error::AlreadyClosed => {
                    info!("WebSocket connection was closed");
                    break;
                },
                tokio_tungstenite::tungstenite::Error::Io(err) if err.kind() == std::io::ErrorKind::BrokenPipe => {
                    info!("Broken pipe error");
                    break;
                },                
                _ => debug!("Error sending message to WebSocket: {}", e),
            },
        }        
    }
}