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
}

impl AppContext {
    pub fn new(streams: HashMap<String,Arc<Mutex<StreamsDef>>>) -> Self {
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
