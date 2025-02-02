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
use rustls_pemfile::{certs, pkcs8_private_keys};
use rustls::{ServerConfig, Certificate, PrivateKey};
use std::io::BufReader;

use log::info;

use serde_json::json;
use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::sync::{Arc, Mutex};
use actix_web_actors::ws;

mod websocketservice;
mod appcontext;
mod rtspclient;
mod streamdef;

use streamdef::StreamsDef;

#[derive(Parser)]
pub struct Opts {
    #[clap(short = 'C')]
    config: String,

    #[clap(short)]
    transport: Option<String>,

    #[arg(short)]
    cert: Option<String>,

    #[arg(short)]
    key: Option<String>,    

    #[arg(short, default_value = "8080")]
    port: u16,    
}

fn load_rustls_config(cert_path: &str, key_path: &str) -> Result<ServerConfig, Error> {
    let cert_file = &mut BufReader::new(File::open(cert_path)?);
    let key_file = &mut BufReader::new(File::open(key_path)?);

    let cert_chain = certs(cert_file)?
        .into_iter()
        .map(Certificate)
        .collect();
    
    let mut keys: Vec<PrivateKey> = pkcs8_private_keys(key_file)?
        .into_iter()
        .map(PrivateKey)
        .collect();

    let config = ServerConfig::builder()
        .with_safe_defaults()
        .with_no_client_auth()
        .with_single_cert(cert_chain, keys.remove(0))?;

    Ok(config)
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
                streams_defs.insert(wsurl, Arc::new(Mutex::new(StreamsDef::new(url))));
            }
        },
        Err(err) => println!("Error reading JSON file: {:?}", err),
    }

    // start the RTSP clients
    let app_context = appcontext::AppContext::new(streams_defs);
    app_context.streams.values().for_each(|streamdef| {
        let stream = streamdef.lock().unwrap();
        tokio::spawn(rtspclient::run(stream.url.clone(), opts.transport.clone(), stream.tx.clone()));
    });

    // Start the Actix web server
    info!("start actix web server");
    let server = HttpServer::new( move || {
        let mut app = App::new().app_data(web::Data::new(app_context.clone()));

        for key in app_context.streams.keys() {
            app = app.route(key, web::get().to(ws_index));
        }

        app.service(version)
            .service(streams)
            .service(logger_level)
            .service(web::redirect("/", "/index.html"))
            .service(Files::new("/", "./www"))
    });

    let server = if let (Some(cert), Some(key)) = (opts.cert, opts.key) {
        let config = load_rustls_config(&cert, &key)
            .expect("Failed to load TLS config");
        server.bind_rustls(format!("0.0.0.0:{}", opts.port), config)
    } else {
        server.bind(format!("0.0.0.0:{}", opts.port))
    };

    server.expect("error").run().await.unwrap();


    info!("Done");
}

// Websocket handler
pub async fn ws_index(req: HttpRequest, stream: web::Payload, data: web::Data<appcontext::AppContext>) -> Result<HttpResponse, actix_web::Error> {
    let app_context = data.get_ref();
    let wsurl = req.path().to_string();
    if app_context.streams.contains_key(&wsurl) {
        let wscontext =  app_context.streams[&wsurl].to_owned();
        let rx = wscontext.lock().unwrap().rx.resubscribe();
        Ok(ws::start(websocketservice::WebsocketService{ wsurl, rx, wscontext }, &req, stream)?)
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
            "count": streamdef.lock().unwrap().count,
        });
    }

    HttpResponse::Ok().json(data)
}

#[get("/api/version")]
async fn version() -> HttpResponse {
    let data = json!("version");

    HttpResponse::Ok().json(data)
}

#[get("/api/log")]
async fn logger_level(query: web::Query<HashMap<String, String>>) -> HttpResponse {
    
    if let Some(level_str) = query.get("level") {
        match level_str.as_str() {
            "Off" => log::set_max_level(log::LevelFilter::Off),
            "Error" => log::set_max_level(log::LevelFilter::Error),
            "Warn" => log::set_max_level(log::LevelFilter::Warn),
            "Info" => log::set_max_level(log::LevelFilter::Info),
            "Debug" => log::set_max_level(log::LevelFilter::Debug),
            "Trace" => log::set_max_level(log::LevelFilter::Trace),
            _ => (),
        }
    }

    let level = log::max_level(); 

    let level_str = match level {
        log::LevelFilter::Off => "Off",
        log::LevelFilter::Error => "Error",
        log::LevelFilter::Warn => "Warn",
        log::LevelFilter::Info => "Info",
        log::LevelFilter::Debug => "Debug",
        log::LevelFilter::Trace => "Trace",
    };

    let data = json!({
        "level": level_str,
    });

    HttpResponse::Ok().json(data)
}