/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use retina::client::{SessionGroup, SetupOptions, Transport};
use retina::codec::{CodecItem, VideoFrame, VideoParameters};
use anyhow::{anyhow, Error};
use log::{debug, error, info};
use serde_json::json;
use std::sync::Arc;
use std::vec;
use tokio::sync::broadcast;
use futures::StreamExt;
use std::io::Cursor;
use std::io::prelude::*;

use crate::streamdef::DataFrame;

pub async fn run(url: url::Url, transport: Option<String>, tx: broadcast::Sender<DataFrame>) -> Result<(), Error> {
    let session_group = Arc::new(SessionGroup::default());
    let r = run_inner(url, transport, session_group.clone(), tx).await;
    if let Err(e) = session_group.await_teardown().await {
        error!("TEARDOWN failed: {}", e);
    }
    r
}

const MARKER: [u8; 4] = [0, 0, 0, 1];

pub fn avcc_to_annex_b(
    data: &[u8]
) -> Result<Vec<u8>, Error> {
    let mut nal_units = vec![];
    let mut data_cursor = Cursor::new(data);
    let mut nal_lenght_bytes = [0u8; 4];
    while let Ok(_) = data_cursor.read_exact(&mut nal_lenght_bytes) {
        let nal_length = u32::from_be_bytes(nal_lenght_bytes) as usize;

        if nal_length == 0 {
            return Err(anyhow!("NalLenghtParseError"));
        }
        let mut nal_unit = vec![0u8; nal_length];
        data_cursor.read_exact(&mut nal_unit)?;

        nal_units.extend_from_slice(&MARKER);
        nal_units.extend_from_slice(&nal_unit);
    }
    Ok(nal_units)
}

fn decode_cfg(data: &[u8]) -> Result<Vec<u8>, Error> {
    let sps_len = u16::from_be_bytes([data[6], data[7]]) as usize;
    let pps_len = u16::from_be_bytes([data[8 + sps_len + 1], data[9 + sps_len + 1]]) as usize;
    if ((8+sps_len) > data.len()) || ((10+sps_len+1+pps_len) > data.len()) {
        return Err(anyhow!("Error decoding cfg"));
    }
    let mut cfg: Vec<u8> = vec![];
    cfg.extend_from_slice(&MARKER);
    cfg.extend_from_slice(&data[8..8+sps_len]);
    cfg.extend_from_slice(&MARKER);
    cfg.extend_from_slice(&data[10+sps_len+1..10+sps_len+1+pps_len]);
    Ok(cfg)
}

fn process_video_frame(m: VideoFrame, video_params: VideoParameters, tx: broadcast::Sender<DataFrame>) {
    debug!(
        "{}: size:{} is_random_access_point:{} has_new_parameters:{}",
        m.timestamp().timestamp(),
        m.data().len(),
        m.is_random_access_point(),
        m.has_new_parameters(),
    );

    let mut metadata = json!({
        "ts":  (m.timestamp().timestamp() as f64)*1000.0,
        "media": "video",
        "codec": video_params.rfc6381_codec(),
    });
    let mut data: Vec<u8> = vec![];
    if m.is_random_access_point() {
        metadata["type"] = "keyframe".into();
        
        let extra_data = video_params.extra_data();
        debug!("extra_data:{:?}", extra_data);
    
        let cfg = decode_cfg(extra_data).unwrap();
        debug!("CFG: {:?}", cfg);    
        data.extend_from_slice(cfg.as_slice());
    }
    let nal_units = avcc_to_annex_b(m.data()).unwrap();
    data.extend_from_slice(nal_units.as_slice());

    let frame = DataFrame {
        metadata,
        data,
    };

    if let Err(e) = tx.send(frame) {
        error!("Error broadcasting message: {}", e);
    }                        
}

async fn run_inner(url: url::Url, transport: Option<String>, session_group: Arc<SessionGroup>, tx: broadcast::Sender<DataFrame>) -> Result<(), Error> {
    let stop = tokio::signal::ctrl_c();

    let mut session = retina::client::Session::describe(
        url.clone(),
        retina::client::SessionOptions::default()
            .session_group(session_group),
    )
    .await?;
    debug!("{:?}", session.streams());

    let video_stream = session
        .streams()
        .iter()
        .position(|s| s.media() == "video" && matches!(s.encoding_name(), "h264" | "h265"))
        .ok_or_else(|| anyhow!("couldn't find video stream"))?;

    let transport_value = match transport {
        Some(t) => t.parse::<Transport>().unwrap(),
        None => Transport::default(), 
    };    
    let options = SetupOptions::transport(SetupOptions::default(), transport_value);
    session
        .setup(video_stream, options)
        .await?;

    let video_params = match session.streams()[video_stream].parameters() {
        Some(retina::codec::ParametersRef::Video(v)) => v.clone(),
        Some(_) => unreachable!(),
        None => unreachable!(),
    };
    info!("video_params:{:?}", video_params);

    let mut videosession = session
        .play(retina::client::PlayOptions::default())
        .await?
        .demuxed()?;

    
    tokio::pin!(stop);
    loop {
        tokio::select! {
            item = videosession.next() => {
                match item.ok_or_else(|| anyhow!("EOF"))?? {
                    CodecItem::VideoFrame(m) => process_video_frame(m, video_params.clone(), tx.clone()),
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
