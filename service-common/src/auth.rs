use async_trait::async_trait;
use drogue_cloud_service_api::auth::{
    AuthenticationClient, AuthenticationClientError, AuthenticationRequest, AuthenticationResponse,
    ErrorInformation,
};
use reqwest::{Client, Response, StatusCode};
use url::Url;

/// An authentication client backed by reqwest.
#[derive(Clone, Debug)]
pub struct ReqwestAuthenticatorClient {
    client: Client,
    auth_service_url: Url,
}

impl ReqwestAuthenticatorClient {
    /// Create a new client instance.
    pub fn new(client: Client, url: Url) -> Self {
        Self {
            client,
            auth_service_url: url,
        }
    }
}

#[async_trait]
impl AuthenticationClient for ReqwestAuthenticatorClient {
    type Error = reqwest::Error;

    async fn authenticate(
        &self,
        request: AuthenticationRequest,
    ) -> Result<AuthenticationResponse, AuthenticationClientError<Self::Error>> {
        let response: Response = self
            .client
            .post(self.auth_service_url.clone())
            .json(&request)
            .send()
            .await
            .map_err(|err| {
                log::warn!("Error while authenticating {:?}: {}", request, err);
                Box::new(err)
            })?;

        match response.status() {
            StatusCode::OK => match response.json::<AuthenticationResponse>().await {
                Ok(result) => {
                    log::debug!("Outcome for {:?} is {:?}", request, result);
                    Ok(result)
                }
                Err(err) => {
                    log::debug!("Authentication failed for {:?}. Result: {:?}", request, err);

                    Err(AuthenticationClientError::Request(format!(
                        "Failed to decode service response: {}",
                        err
                    )))
                }
            },
            code => match response.json::<ErrorInformation>().await {
                Ok(result) => {
                    log::debug!("Service reported error ({}): {}", code, result);
                    Err(AuthenticationClientError::Service(result))
                }
                Err(err) => {
                    log::debug!(
                        "Authentication failed ({}) for {:?}. Result couldn't be decoded: {:?}",
                        code,
                        request,
                        err
                    );
                    Err(AuthenticationClientError::Request(format!(
                        "Failed to decode service error response: {}",
                        err
                    )))
                }
            },
        }
    }
}
