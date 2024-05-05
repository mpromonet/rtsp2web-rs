/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/


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

pub struct MyWs {
    rx: broadcast::Receiver<Frame>,
}

impl MyWs {
    pub fn new(rx: broadcast::Receiver<Frame>) -> Self {
        Self { rx }
    }
}

impl Clone for MyWs {
    fn clone(&self) -> Self {
        Self {
            rx: self.rx.resubscribe(),
        }
    }
}

impl Actor for MyWs {
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

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for MyWs {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            _ => (),
        }
    }
}

impl StreamHandler<Result<Frame, BroadcastStreamRecvError>> for MyWs {
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

pub async fn ws_index(req: HttpRequest, stream: web::Payload, data: web::Data<MyWs>) -> Result<HttpResponse, actix_web::Error> {
    let myws = data.get_ref();
    ws::start(myws.clone(), &req, stream)
}
