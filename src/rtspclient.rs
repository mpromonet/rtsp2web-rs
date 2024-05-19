/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use retina::client::{SessionGroup, SetupOptions, Transport};
use retina::codec::{CodecItem, VideoFrame};
use anyhow::{anyhow, Error};
use log::{debug, error, info};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::broadcast;
use futures::StreamExt;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::streamdef::DataFrame;

pub async fn run(url: url::Url, transport: Option<String>, tx: broadcast::Sender<DataFrame>) -> Result<(), Error> {
    let session_group = Arc::new(SessionGroup::default());
    let r = run_inner(url, transport, session_group.clone(), tx).await;
    if let Err(e) = session_group.await_teardown().await {
        error!("TEARDOWN failed: {}", e);
    }
    r
}

fn process_video_frame(m: VideoFrame, codec: &str, cfg: &[u8], tx: broadcast::Sender<DataFrame>) {
    debug!(
        "{}: size:{} is_random_access_point:{} has_new_parameters:{}",
        m.timestamp().timestamp(),
        m.data().len(),
        m.is_random_access_point(),
        m.has_new_parameters(),
    );
    let now = SystemTime::now();
    let since_the_epoch = now
        .duration_since(UNIX_EPOCH).unwrap();
    let in_us = since_the_epoch.as_millis();

    let mut metadata = json!({
        "ts": in_us,
        "media": "video",
        "codec": codec,
    });
    let mut data: Vec<u8> = vec![];
    if m.is_random_access_point() {
        metadata["type"] = "keyframe".into();
        data.extend_from_slice(&cfg);
    }
    let mut framedata = m.data().to_vec();
    if framedata.len() > 3 {
        framedata[0] = 0;
        framedata[1] = 0;
        framedata[2] = 0;
        framedata[3] = 1;
    }
    data.extend_from_slice(framedata.as_slice());

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
        url,
        retina::client::SessionOptions::default()
            .session_group(session_group),
    )
    .await?;
    debug!("{:?}", session.streams());

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

    let video_params = match session.streams()[video_stream].parameters() {
        Some(retina::codec::ParametersRef::Video(v)) => v.clone(),
        Some(_) => unreachable!(),
        None => unreachable!(),
    };
    debug!("video_params:{:?}", video_params);
    let extra_data = video_params.extra_data();
    debug!("extra_data:{:?}", extra_data);

    let sps_position = extra_data.iter().position(|&nal| nal & 0x1F == 7);
    let pps_position = extra_data.iter().position(|&nal| nal & 0x1F == 8);

    let mut cfg: Vec<u8> = vec![];
    if let (Some(sps), Some(pps)) = (sps_position, pps_position) {
        if sps < pps {
            cfg = vec![0x00, 0x00, 0x00, 0x01];
            cfg.extend_from_slice(&extra_data[sps..pps]);
            cfg.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]);
            cfg.extend_from_slice(&extra_data[pps..]);
            info!("CFG: {:?}", cfg);
        }
    }
    let transport_value = match transport {
        Some(t) => t.parse::<Transport>().unwrap(),
        None => Transport::default(), 
    };    
    let options = SetupOptions::transport(SetupOptions::default(), transport_value);
    session
        .setup(video_stream, options)
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
                    CodecItem::VideoFrame(m) => process_video_frame(m, video_params.rfc6381_codec(), cfg.as_slice(), tx.clone()),
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
