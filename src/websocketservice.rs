/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/



use std::sync::Arc;
use std::sync::Mutex;

use actix::{Actor, AsyncContext, StreamHandler};
use actix_web_actors::ws;
use log::{error, info};
use tokio::sync::broadcast;
use tokio::sync::oneshot;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use crate::streamdef::DataFrame;
use crate::streamdef::StreamsDef;

pub struct WebsocketService {
    pub rx: broadcast::Receiver<DataFrame>,
    pub wsurl: String,
    pub wscontext: Arc<Mutex<StreamsDef>>,
}

impl Actor for WebsocketService {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Websocket {} connected", self.wsurl);
        let rx = self.rx.resubscribe();
        let stream = tokio_stream::wrappers::BroadcastStream::<DataFrame>::new(rx);
        ctx.add_stream(stream);

        let mut wscontext = self.wscontext.lock().unwrap();
        wscontext.count += 1;

        let should_start = wscontext.count == 1
            || wscontext
                .task
                .as_ref()
                .map(|task| task.is_finished())
                .unwrap_or(true);

        if should_start {
            let (stop_tx, stop_rx) = oneshot::channel();
            let url = wscontext.url.clone();
            let transport = wscontext.transport.clone();
            let tx = wscontext.tx.clone();
            let wsurl = self.wsurl.clone();

            wscontext.stop_tx = Some(stop_tx);
            wscontext.task = Some(tokio::spawn(async move {
                info!("RTSP {} started", wsurl);
                if let Err(e) = crate::rtspclient::run_until(url, transport, tx, stop_rx).await {
                    error!("RTSP {} exited with error: {}", wsurl, e);
                }
                info!("RTSP {} stopped", wsurl);
            }));
        }
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        info!("Websocket {} disconnected", self.wsurl);
        let mut wscontext = self.wscontext.lock().unwrap();

        if wscontext.count > 0 {
            wscontext.count -= 1;
        }

        if wscontext.count == 0 {
            if let Some(stop_tx) = wscontext.stop_tx.take() {
                let _ = stop_tx.send(());
            }
            wscontext.task.take();
        }
    }    
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebsocketService {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        if let Ok(ws::Message::Ping(msg)) = msg {
            ctx.pong(&msg);
        }
    }
}

impl StreamHandler<Result<DataFrame, BroadcastStreamRecvError>> for WebsocketService {
    fn handle(&mut self, msg: Result<DataFrame, BroadcastStreamRecvError>, ctx: &mut Self::Context) {
        if let Ok(msg) = msg {
            ctx.text(serde_json::to_string(&msg.metadata).unwrap());
            ctx.binary(msg.data);
        }
    }
}
