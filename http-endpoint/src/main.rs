mod command;
mod downstream;
mod telemetry;
mod ttn;
mod x509;

#[cfg(feature = "rustls")]
use actix_tls::rustls::Session;
use actix_web::{
    get, middleware,
    web::{self, Data},
    App, HttpResponse, HttpServer, Responder,
};
use dotenv::dotenv;
use drogue_cloud_endpoint_common::{
    auth::{AuthConfig, DeviceAuthenticator},
    command_endpoint::{CommandServer, CommandServerConfig},
    downstream::DownstreamSender,
};
use drogue_cloud_service_common::openid;
use envconfig::Envconfig;
use futures::future;
use serde_json::json;
use std::convert::TryInto;
use std::ops::DerefMut;

drogue_cloud_endpoint_common::retriever!();

#[cfg(feature = "rustls")]
drogue_cloud_endpoint_common::retriever_rustls!(actix_tls::rustls::TlsStream<T>);

#[cfg(feature = "openssl")]
drogue_cloud_endpoint_common::retriever_openssl!(actix_tls::openssl::SslStream<T>);

#[cfg(feature = "ntex")]
retriever_none!(ntex::rt::net::TcpStream);

#[derive(Envconfig, Clone, Debug)]
struct Config {
    #[envconfig(from = "MAX_JSON_PAYLOAD_SIZE", default = "65536")]
    pub max_json_payload_size: usize,
    #[envconfig(from = "MAX_PAYLOAD_SIZE", default = "65536")]
    pub max_payload_size: usize,
    #[envconfig(from = "BIND_ADDR", default = "127.0.0.1:8443")]
    pub bind_addr: String,
    #[envconfig(from = "HEALTH_BIND_ADDR", default = "127.0.0.1:8081")]
    pub health_bind_addr: String,
    #[envconfig(from = "AUTH_SERVICE_URL")]
    pub auth_service_url: Option<String>,
    #[envconfig(from = "DISABLE_TLS", default = "false")]
    pub disable_tls: bool,
    #[envconfig(from = "CERT_BUNDLE_FILE")]
    pub cert_file: Option<String>,
    #[envconfig(from = "KEY_FILE")]
    pub key_file: Option<String>,
}

#[get("/")]
async fn index() -> impl Responder {
    HttpResponse::Ok().json(json!({"success": true}))
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().finish()
}

#[actix_web::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    dotenv().ok();

    log::info!("Starting HTTP service endpoint");

    let sender = DownstreamSender::new()?;

    let config = Config::init_from_env()?;
    let max_payload_size = config.max_payload_size;
    let max_json_payload_size = config.max_json_payload_size;

    // OpenIdConnect

    let config: AuthConfig = AuthConfig::init_from_env()?;
    let (client, scopes) = (Some(openid::create_client(&config).await?), config.scopes);
    let service_authenticator = openid::Authenticator::new(client, scopes);

    let mut device_authenticator: DeviceAuthenticator = AuthConfig::init_from_env()?.try_into()?;
    device_authenticator
        .client
        .set_service_token(service_authenticator.bearer);

    let http_server = HttpServer::new(move || {
        let app = App::new()
            .wrap(middleware::Logger::default())
            .app_data(web::PayloadConfig::new(max_payload_size))
            .data(web::JsonConfig::default().limit(max_json_payload_size))
            .data(sender.clone());

        let app = app
            .app_data(Data::new(device_authenticator.clone()))
            .app_data(Data::new(service_authenticator.clone()));

        app.service(index)
            // the standard endpoint
            .service(
                web::scope("/v1")
                    .service(telemetry::publish_plain)
                    .service(telemetry::publish_tail),
            )
            // The Things Network variant
            .service(web::scope("/ttn").service(ttn::publish))
            //fixme : bind to a different port
            .service(health)
    })
    .on_connect(|con, ext| {
        if let Some(cert) = x509::from_socket(con) {
            if !cert.0.is_empty() {
                ext.insert(cert);
            }
        }
    });

    let http_server = match (config.disable_tls, config.key_file, config.cert_file) {
        (false, Some(key), Some(cert)) => {
            if cfg!(feature = "openssl") {
                use open_ssl::ssl;
                let method = ssl::SslMethod::tls_server();
                let mut builder = ssl::SslAcceptor::mozilla_intermediate(method)?;
                builder.set_private_key_file(key, ssl::SslFiletype::PEM)?;
                builder.set_certificate_chain_file(cert)?;
                // we ask for client certificates, but don't enforce them
                builder.set_verify_callback(ssl::SslVerifyMode::PEER, |_, ctx| {
                    log::debug!(
                        "Accepting client certificates: {:?}",
                        ctx.current_cert()
                            .map(|cert| format!("{:?}", cert.subject_name()))
                            .unwrap_or_else(|| "<unknown>".into())
                    );
                    true
                });

                http_server.bind_openssl(config.bind_addr, builder)?
            } else {
                panic!("TLS is required, but no TLS implementation enabled")
            }
        }
        (true, None, None) => http_server.bind(config.bind_addr)?,
        (false, _, _) => panic!("Wrong TLS configuration: TLS enabled, but key or cert is missing"),
        (true, Some(_), _) | (true, _, Some(_)) => {
            // the TLS configuration must be consistent, to prevent configuration errors.
            panic!("Wrong TLS configuration: key or cert specified, but TLS is disabled")
        }
    };

    let http_server = http_server.run();

    let mut command_server: CommandServer = CommandServerConfig::init_from_env()?.try_into()?;

    future::try_join(command_server.deref_mut(), http_server).await?;

    // fixme
    //
    // let health_server = HttpServer::new(move || App::new().service(health))
    //     .bind(config.health_bind_addr)?
    //     .run();
    //
    // future::try_join(app_server, health_server).await?;

    Ok(())
}
