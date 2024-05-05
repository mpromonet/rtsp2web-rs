/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use anyhow::{anyhow, Error, bail};
use actix_files::Files;
use actix_web::{get, web, App, HttpServer, HttpResponse};
use clap::Parser;
use futures::StreamExt;
use log::{error, info, debug};
use retina::client::{SessionGroup, SetupOptions};
use retina::codec::{CodecItem, VideoFrame};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::broadcast;

mod wsservice;

#[derive(Parser)]
pub struct Opts {
    /// `rtsp://` URL to connect to.
    #[clap(long)]
    url: url::Url,
}

pub async fn run(opts: Opts, tx: broadcast::Sender<wsservice::Frame>) -> Result<(), Error> {
    let session_group = Arc::new(SessionGroup::default());
    let r = run_inner(opts, session_group.clone(), tx).await;
    if let Err(e) = session_group.await_teardown().await {
        error!("TEARDOWN failed: {}", e);
    }
    r
}

async fn run_inner(opts: Opts, session_group: Arc<SessionGroup>, tx: broadcast::Sender<wsservice::Frame>) -> Result<(), Error> {
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

    let video_params = match session.streams()[video_stream].parameters() {
        Some(retina::codec::ParametersRef::Video(v)) => v.clone(),
        Some(_) => unreachable!(),
        None => unreachable!(),
    };
    info!("video_params:{:?}", video_params);

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
                            "{}: size:{} is_random_access_point:{} has_new_parameters:{}",
                            m.timestamp().timestamp(),
                            m.data().len(),
                            m.is_random_access_point(),
                            m.has_new_parameters(),
                        );

                        let mut metadata = json!({
                            "ts": m.timestamp().timestamp(),
                            "media": "video",
                            "codec": video_params.rfc6381_codec(),
                        });
                        if m.is_random_access_point() {
                            metadata["type"] = "keyframe".into();
                        }

                        let frame = wsservice::Frame {
                            metadata,
                            data: convert_h264(m)?.into(),
                        };

                        if let Err(e) = tx.send(frame) {
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
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    // Create a broadcast channel to send video frames to the WebSocket server
    let (tx, rx) = broadcast::channel::<wsservice::Frame>(100);
    let myws = wsservice::MyWs::new(rx);

    // Start the Actix web server
    info!("start actix web server");
    tokio::spawn(async {
        HttpServer::new( move || {
            App::new().app_data(web::Data::new(myws.clone()))
                .route("/ws", web::get().to(wsservice::ws_index))
                .service(version)
                .service(streams)
                .service(web::redirect("/", "/index.html"))
                .service(Files::new("/", "./www").show_files_listing())
        })
        .bind(("0.0.0.0", 8080)).unwrap()
        .run()
        .await
        .unwrap();
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

#[get("/api/streams")]
async fn streams() -> HttpResponse {
    let data = json!({
        "/ws": "stream1",
    });

    HttpResponse::Ok().json(data)
}

#[get("/api/version")]
async fn version() -> HttpResponse {
    let data = json!("version");

    HttpResponse::Ok().json(data)
}

fn convert_h264(frame: VideoFrame) -> Result<Vec<u8>, Error> {
    let mut data = frame.into_data();
    let mut i = 0;
    while i < data.len() - 3 {
        // Replace each NAL's length with the Annex B start code b"\x00\x00\x00\x01".
        let len = u32::from_be_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]) as usize;
        data[i] = 0;
        data[i + 1] = 0;
        data[i + 2] = 0;
        data[i + 3] = 1;
        i += 4 + len;
        if i > data.len() {
            bail!("partial NAL body");
        }
    }
    if i < data.len() {
        bail!("partial NAL length");
    }
    Ok(data)
}