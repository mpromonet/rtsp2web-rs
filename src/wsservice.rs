/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/


use std::collections::HashMap;

use actix::{Actor, AsyncContext, StreamHandler};
use actix_web::{web, HttpRequest, HttpResponse};
use actix_web_actors::ws;
use log::info;
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;

#[derive(Clone)]
pub struct Frame {
    pub metadata: serde_json::Value,
    pub data: Vec<u8>,
}

pub struct StreamsDef {
    pub url: url::Url,
    pub tx: broadcast::Sender<Frame>,
    pub rx: broadcast::Receiver<Frame>,
}

impl Clone for StreamsDef {
    fn clone(&self) -> Self {
        Self {
            url: self.url.clone(),
            tx: self.tx.clone(),
            rx: self.rx.resubscribe(),
        }
    }
}

impl StreamsDef {
    pub fn new(url: url::Url) -> Self {
        let url = url;
        let (tx, rx) = broadcast::channel::<Frame>(100);

        StreamsDef { url, tx,  rx }
    }
}

pub struct AppContext {
    pub streams: HashMap<&'static str,StreamsDef>,
}

impl AppContext {
    pub fn new(streams: HashMap<&'static str,StreamsDef>) -> Self {
        Self { streams }
    }
}

impl Clone for AppContext {
    fn clone(&self) -> Self {
        Self {
            streams: self.streams.clone(),
        }
    }
}

struct MyWebsocket {
    pub rx: broadcast::Receiver<Frame>,
}

impl Actor for MyWebsocket {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Websocket connected");
        let rx = self.rx.resubscribe();
        let stream = tokio_stream::wrappers::BroadcastStream::<Frame>::new(rx);
        ctx.add_stream(stream);
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        info!("Websocket disconnected");
    }    
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWebsocket {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            _ => (),
        }
    }
}

impl StreamHandler<Result<Frame, BroadcastStreamRecvError>> for MyWebsocket {
    fn handle(&mut self, msg: Result<Frame, BroadcastStreamRecvError>, ctx: &mut Self::Context) {
        match msg {
            Ok(msg) => {
                ctx.text(serde_json::to_string(&msg.metadata).unwrap());
                ctx.binary(msg.data);
            },
            _ => (),
        }
    }
}

pub async fn ws_index(req: HttpRequest, stream: web::Payload, data: web::Data<AppContext>) -> Result<HttpResponse, actix_web::Error> {
    let myws = data.get_ref();
    let rx = myws.streams[req.path()].rx.resubscribe();
    ws::start(MyWebsocket{ rx }, &req, stream)
}
