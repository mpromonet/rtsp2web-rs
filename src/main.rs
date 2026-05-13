/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use anyhow::{anyhow, Error};
use actix_files::Files;
use actix_web::{get, web, App, HttpServer, HttpRequest, HttpResponse};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;
use clap::Parser;
use rustls_pemfile::{certs, pkcs8_private_keys};
use rustls::ServerConfig;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::io::BufReader;

use log::{error, info, warn};

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
mod webtransportservice;

use streamdef::StreamsDef;

#[derive(OpenApi)]
#[openapi(
    paths(version, streams, quic_info, logger_level),
    info(
        title = "rtsp2web-rs",
        description = "RTSP to WebSocket/WebTransport proxy",
        version = "0.1.0"
    )
)]
struct ApiDoc;

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

    #[arg(short = 'q')]
    quic_port: Option<u16>,
}

fn load_rustls_config(cert_path: &str, key_path: &str) -> Result<ServerConfig, Error> {

    let cert_file = File::open(cert_path)?;
    let mut reader = BufReader::new(cert_file);
    let cert_chain: Vec<CertificateDer<'static>> = certs(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| std::io::Error::other("Failed to load certificates"))?;
    if cert_chain.is_empty() {
        return Err(anyhow!("No certificates found in {}", cert_path));
    }

    // For keys:
    let key_file = File::open(key_path)?;
    let mut reader = BufReader::new(key_file);
    let mut keys: Vec<PrivateKeyDer<'static>> = pkcs8_private_keys(&mut reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| std::io::Error::other("Failed to load private key"))?
        .into_iter()
        .map(PrivateKeyDer::from)
        .collect();
    if keys.is_empty() {
        return Err(anyhow!("No PKCS#8 private key found in {}", key_path));
    }
        
    let config = ServerConfig::builder()
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
    // Both ring and aws-lc-rs are compiled in; install ring as the explicit
    // process-level rustls CryptoProvider before any TLS stack is initialised.
    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    let opts = Opts::parse();

    let mut streams_defs = HashMap::new();
    let data = match read_json_file(opts.config.as_str()) {
        Ok(data) => data,
        Err(err) => {
            error!("Error reading JSON file {}: {}", opts.config, err);
            return;
        }
    };

    let Some(urls) = data["urls"].as_object() else {
        error!("Invalid config {}: missing object field 'urls'", opts.config);
        return;
    };

    for (key, value) in urls {
        let Some(video_url) = value["video"].as_str() else {
            warn!("Skipping stream '{}' because 'video' is missing or not a string", key);
            continue;
        };

        match url::Url::parse(video_url) {
            Ok(url) => {
                let wsurl = "/".to_string() + key;
                streams_defs.insert(wsurl, Arc::new(Mutex::new(StreamsDef::new(url, opts.transport.clone()))));
            }
            Err(err) => {
                warn!("Skipping stream '{}' with invalid URL '{}': {}", key, video_url, err);
            }
        }
    }

    if streams_defs.is_empty() {
        error!("No valid streams configured in {}", opts.config);
        return;
    }

    // Build a 14-day self-signed identity for the QUIC endpoint.
    // Short validity enables `serverCertificateHashes` in Chrome, allowing
    // WebTransport to work with a self-signed cert for any origin.
    let (quic_identity, cert_fingerprint) = if opts.quic_port.is_some() {
        // Include the HTTPS cert's CN as an extra SAN hint if available.
        let extra: Vec<String> = opts.cert.as_ref()
            .and_then(|c| {
                let f = std::fs::File::open(c).ok()?;
                let mut r = std::io::BufReader::new(f);
                let first_cert = { rustls_pemfile::certs(&mut r).next() };
                first_cert?.ok().and_then(|_der| {
                    // best-effort: pull the hostname from the system
                    hostname::get().ok()
                        .and_then(|h| h.into_string().ok())
                        .map(|h| vec![h])
                })
            })
            .unwrap_or_default();
        match webtransportservice::build_identity(&extra) {
            Ok((id, fp)) => {
                info!("WebTransport cert fingerprint: {}", fp.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"));
                (Some(id), Some(fp))
            }
            Err(e) => {
                error!("Failed to build WebTransport identity: {e}");
                (None, None)
            }
        }
    } else {
        (None, None)
    };

    let app_context = appcontext::AppContext::new(streams_defs, opts.quic_port, cert_fingerprint);

    // Start the WebTransport (QUIC) server if --quic-port is set.
    if let (Some(quic_port), Some(identity)) = (opts.quic_port, quic_identity) {
        let app_ctx = app_context.clone();
        tokio::spawn(async move {
            if let Err(e) = webtransportservice::run(app_ctx, identity, quic_port).await {
                error!("WebTransport server exited with error: {e}");
            }
        });
    }

    // Start the Actix web server
    info!("start actix web server");
    let server = HttpServer::new( move || {
        let mut app = App::new().app_data(web::Data::new(app_context.clone()));

        for key in app_context.streams.keys() {
            app = app.route(key, web::get().to(ws_index));
        }

        app.service(SwaggerUi::new("/swagger-ui/{_:.*}").url("/api-docs/openapi.json", ApiDoc::openapi()))
            .service(version)
            .service(streams)
            .service(quic_info)
            .service(logger_level)
            .service(web::redirect("/", "/index.html"))
            .service(Files::new("/", "./www"))
    });

    let server = if let (Some(cert), Some(key)) = (opts.cert, opts.key) {
        let rustls_config = load_rustls_config(&cert, &key)
            .expect("Failed to load TLS config");

        server.bind_rustls_0_23(format!("0.0.0.0:{}", opts.port), rustls_config)
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
    if let Some(wscontext) = app_context.streams.get(&wsurl) {
        let wscontext = wscontext.to_owned();
        Ok(ws::start(websocketservice::WebsocketService{ wsurl, wscontext }, &req, stream)?)
    } else {
        Ok(HttpResponse::NotFound().finish())
    }
}

#[utoipa::path(
    get,
    path = "/api/streams",
    responses(
        (status = 200, description = "List of configured streams with connection counts")
    )
)]
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

#[utoipa::path(
    get,
    path = "/api/quic",
    responses(
        (status = 200, description = "QUIC/WebTransport port and certificate fingerprint, or null if disabled")
    )
)]
#[get("/api/quic")]
async fn quic_info(data: web::Data<appcontext::AppContext>) -> HttpResponse {
    let ctx = data.get_ref();
    let response = match (ctx.quic_port, ctx.cert_fingerprint.as_deref()) {
        (Some(port), Some(fp)) => serde_json::json!({ "port": port, "fingerprint": fp }),
        _ => serde_json::json!(null),
    };
    HttpResponse::Ok().json(response)
}

#[utoipa::path(
    get,
    path = "/api/version",
    responses(
        (status = 200, description = "Git version tag of the running binary")
    )
)]
#[get("/api/version")]
async fn version() -> HttpResponse {
    let data = json!(env!("GIT_VERSION"));
    HttpResponse::Ok().json(data)
}

#[utoipa::path(
    get,
    path = "/api/log",
    params(
        ("level" = Option<String>, Query, description = "Set log level: Off, Error, Warn, Info, Debug, Trace")
    ),
    responses(
        (status = 200, description = "Current log level")
    )
)]
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
