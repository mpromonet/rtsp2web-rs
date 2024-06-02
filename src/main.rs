/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use anyhow::Error;
use actix_files::Files;
use actix_web::{get, web, App, HttpServer, HttpRequest, HttpResponse};
use clap::Parser;

use log::info;

use serde_json::json;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use actix_web_actors::ws;

mod websocketservice;
mod appcontext;
mod rtspclient;
mod streamdef;

use streamdef::StreamsDef;

#[derive(Parser)]
pub struct Opts {
    #[clap(short)]
    config: String,

    #[clap(short)]
    transport: Option<String>,
}

fn read_json_file(file_path: &str) -> Result<serde_json::Value, Error> {
    let mut file = File::open(file_path)?;
    let mut contents = String::new();
    file.read_to_string(&mut contents)?;
    let data = serde_json::from_str(&contents)?;
    Ok(data)
}



#[tokio::main]
async fn main() {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let opts = Opts::parse();

    let mut streams_defs = HashMap::new();
    match read_json_file(opts.config.as_str()) {
        Ok(data) => {
            let urls = data["urls"].as_object().unwrap();
            for (key, value) in urls.into_iter() {
                let url = url::Url::parse(value["video"].as_str().unwrap()).unwrap().clone();
                let wsurl = "/".to_string() + key;
                streams_defs.insert(wsurl, StreamsDef::new(url));
            }
        },
        Err(err) => println!("Error reading JSON file: {:?}", err),
    }

    // start the RTSP clients
    let app_context = appcontext::AppContext::new(streams_defs);
    app_context.streams.values().for_each(|streamdef| {
        tokio::spawn(rtspclient::run(streamdef.url.clone(), opts.transport.clone(), streamdef.tx.clone()));
    });

    // Start the Actix web server
    info!("start actix web server");
    HttpServer::new( move || {
        let mut app = App::new().app_data(web::Data::new(app_context.clone()));

        for key in app_context.streams.keys() {
            app = app.route(key, web::get().to(ws_index));
        }

        app.service(version)
            .service(streams)
            .service(web::redirect("/", "/index.html"))
            .service(Files::new("/", "./www"))
    })
    .bind(("0.0.0.0", 8080)).unwrap()
    .run()
    .await
    .unwrap();


    info!("Done");
}

// Websocket handler
pub async fn ws_index(req: HttpRequest, stream: web::Payload, data: web::Data<appcontext::AppContext>) -> Result<HttpResponse, actix_web::Error> {
    let app_context = data.get_ref();
    let wsurl = req.path().to_string();
    if app_context.streams.contains_key(&wsurl) {
        let rx = app_context.streams[&wsurl].rx.resubscribe();
        Ok(ws::start(websocketservice::WebsocketService{ wsurl, rx }, &req, stream)?)
    } else {
        Ok(HttpResponse::NotFound().finish())
    }
}

#[get("/api/streams")]
async fn streams(data: web::Data<appcontext::AppContext>) -> HttpResponse {
    let app_context = data.get_ref();
    let mut data = json!({});
    for (key, streamdef) in &app_context.streams {
        data[key] = json!({
            "url": streamdef.url.to_string(),
        });
    }

    HttpResponse::Ok().json(data)
}

#[get("/api/version")]
async fn version() -> HttpResponse {
    let data = json!("version");

    HttpResponse::Ok().json(data)
}
