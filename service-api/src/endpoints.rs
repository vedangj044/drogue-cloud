use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HttpEndpoint {
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct MqttEndpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct RegistryEndpoint {
    pub url: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Endpoints {
    pub http: Option<HttpEndpoint>,
    pub mqtt: Option<MqttEndpoint>,
    pub sso: Option<String>,
    pub issuer_url: Option<String>,
    pub redirect_url: Option<String>,
    pub registry: Option<RegistryEndpoint>,
    pub command_url: Option<String>,
    #[serde(default)]
    pub demos: Vec<(String, String)>,
}
