/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use retina::client::{SessionGroup, SetupOptions, Transport};
use retina::codec::{CodecItem, FrameFormat, VideoFrame, VideoParameters};
use anyhow::{anyhow, Error};
use log::{debug, error, info};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::broadcast;
use tokio::sync::oneshot;
use futures::StreamExt;
use std::future::Future;

use crate::streamdef::DataFrame;

pub async fn run_until(
    url: url::Url,
    transport: Option<String>,
    tx: broadcast::Sender<DataFrame>,
    stop: oneshot::Receiver<()>,
) -> Result<(), Error> {
    let session_group = Arc::new(SessionGroup::default());
    let r = run_inner(url, transport, session_group.clone(), tx, async move {
        let _ = stop.await;
    })
    .await;
    if let Err(e) = session_group.await_teardown().await {
        error!("TEARDOWN failed: {}", e);
    }
    r
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
            
        let cfg = video_params.extra_data();
        data.extend_from_slice(cfg);
    }
    data.extend_from_slice(m.data());

    let frame = DataFrame {
        metadata,
        data,
    };

    if let Err(e) = tx.send(frame) {
        error!("Error broadcasting message: {}", e);
    }                        
}

async fn run_inner<Stop>(
    url: url::Url,
    transport: Option<String>,
    session_group: Arc<SessionGroup>,
    tx: broadcast::Sender<DataFrame>,
    stop: Stop,
) -> Result<(), Error>
where
    Stop: Future<Output = ()>,
{

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

    let options = SetupOptions::frame_format(SetupOptions::default(), FrameFormat::SIMPLE);
    let options = SetupOptions::transport(options, transport_value);
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
