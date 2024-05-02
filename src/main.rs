use anyhow::{anyhow, Error};
use clap::Parser;
use futures::{StreamExt, SinkExt};
use log::{error, info};
use retina::client::{SessionGroup, SetupOptions};
use retina::codec::CodecItem;
use std::sync::Arc;

use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::protocol::Message;

#[derive(Parser)]
pub struct Opts {
    /// `rtsp://` URL to connect to.
    #[clap(long)]
    url: url::Url,
}

pub async fn run(opts: Opts) -> Result<(), Error> {
    let session_group = Arc::new(SessionGroup::default());
    let r = run_inner(opts, session_group.clone()).await;
    if let Err(e) = session_group.await_teardown().await {
        error!("TEARDOWN failed: {}", e);
    }
    r
}

async fn run_inner(opts: Opts, session_group: Arc<SessionGroup>) -> Result<(), Error> {
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
                        info!(
                            "{}: size:{}\n",
                            m.timestamp().timestamp(),
                            m.data().len(),
                        );
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

    // Start the WebSocket server
    info!("start websocket");
    let listener = TcpListener::bind("0.0.0.0:9001").await.unwrap();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(accept_connection(stream));
        }
    });

    // Start the RTSP client
    info!("start rtsp client");
    if let Err(e) = {
        let opts = Opts::parse();
        run(opts).await
    } {
        error!("Fatal: {}", itertools::join(e.chain(), "\ncaused by: "));
        std::process::exit(1);
    }


    info!("Done");
}

async fn accept_connection(stream: tokio::net::TcpStream) {
    let mut websocket = accept_async(stream).await.unwrap();

    while let Some(msg) = websocket.next().await {
        let msg = msg.unwrap();
        if msg.is_text() || msg.is_binary() {
            websocket.send(Message::Text("Hello".to_string())).await.unwrap();
        }
    }
}