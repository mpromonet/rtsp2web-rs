/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/



use actix::{Actor, AsyncContext, StreamHandler};
use actix_web_actors::ws;
use log::info;
use tokio::sync::broadcast;
use tokio_stream::wrappers::errors::BroadcastStreamRecvError;
use crate::streamdef::DataFrame;

pub struct WebsocketService {
    pub rx: broadcast::Receiver<DataFrame>,
    pub wsurl: String,
}

impl Actor for WebsocketService {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        info!("Websocket {} connected", self.wsurl);
        let rx = self.rx.resubscribe();
        let stream = tokio_stream::wrappers::BroadcastStream::<DataFrame>::new(rx);
        ctx.add_stream(stream);
    }

    fn stopped(&mut self, _ctx: &mut Self::Context) {
        info!("Websocket {} disconnected", self.wsurl);
    }    
}

impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebsocketService {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            _ => (),
        }
    }
}

impl StreamHandler<Result<DataFrame, BroadcastStreamRecvError>> for WebsocketService {
    fn handle(&mut self, msg: Result<DataFrame, BroadcastStreamRecvError>, ctx: &mut Self::Context) {
        match msg {
            Ok(msg) => {
                ctx.text(serde_json::to_string(&msg.metadata).unwrap());
                ctx.binary(msg.data);
            },
            _ => (),
        }
    }
}
