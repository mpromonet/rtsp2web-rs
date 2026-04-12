/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use tokio::sync::broadcast;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

#[derive(Clone)]
pub struct DataFrame {
    pub metadata: serde_json::Value,
    pub data: Vec<u8>,
}

pub struct StreamsDef {
    pub url: url::Url,
    pub transport: Option<String>,
    pub tx: broadcast::Sender<DataFrame>,
    pub rx: broadcast::Receiver<DataFrame>,
    pub count: u32,
    pub stop_tx: Option<oneshot::Sender<()>>,
    pub task: Option<JoinHandle<()>>,
}

impl StreamsDef {
    pub fn new(url: url::Url, transport: Option<String>) -> Self {
        let (tx, rx) = broadcast::channel::<DataFrame>(100);

        Self {
            url,
            transport,
            tx,
            rx,
            count: 0,
            stop_tx: None,
            task: None,
        }
    }
}