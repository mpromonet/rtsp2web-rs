/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/


use std::{collections::HashMap, sync::{Arc, Mutex}};
use crate::streamdef::StreamsDef;

pub struct AppContext {
    pub streams: HashMap<String,Arc<Mutex<StreamsDef>>>,
    pub quic_port: Option<u16>,
    pub cert_fingerprint: Option<Vec<u8>>,
}

impl AppContext {
    pub fn new(
        streams: HashMap<String,Arc<Mutex<StreamsDef>>>,
        quic_port: Option<u16>,
        cert_fingerprint: Option<Vec<u8>>,
    ) -> Self {
        Self { streams, quic_port, cert_fingerprint }
    }
}

impl Clone for AppContext {
    fn clone(&self) -> Self {
        Self {
            streams: self.streams.clone(),
            quic_port: self.quic_port,
            cert_fingerprint: self.cert_fingerprint.clone(),
        }
    }
}
