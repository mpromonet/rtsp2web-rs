/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use tokio::sync::broadcast;
use std::collections::HashMap;

#[derive(Clone)]
pub struct DataFrame {
    pub metadata: serde_json::Value,
    pub data: Vec<u8>,
}

pub struct StreamsDef {
    pub url: url::Url,
    pub tx: broadcast::Sender<DataFrame>,
    pub rx: broadcast::Receiver<DataFrame>,
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
        let (tx, rx) = broadcast::channel::<DataFrame>(100);

        StreamsDef { url, tx,  rx }
    }
}

pub struct AppContext {
    pub streams: HashMap<String,StreamsDef>,
}

impl AppContext {
    pub fn new(streams: HashMap<String,StreamsDef>) -> Self {
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
