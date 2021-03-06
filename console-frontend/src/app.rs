use crate::backend::{Backend, BackendInformation, Token};
use crate::error::error;
use crate::{components::placeholder::Placeholder, examples::Examples, index::Index, spy::Spy};
use anyhow::Error;
use chrono::{DateTime, Utc};
use drogue_cloud_console_common::UserInfo;
use patternfly_yew::*;
use std::time::Duration;
use url::Url;
use wasm_bindgen::JsValue;
use yew::{
    format::{Json, Nothing},
    prelude::*,
    services::{
        fetch::{Request, *},
        timeout::*,
    },
    utils::window,
};
use yew_router::prelude::*;

#[derive(Switch, Debug, Clone, PartialEq)]
pub enum AppRoute {
    #[to = "/spy"]
    Spy,
    #[to = "/examples"]
    Examples,
    #[to = "/"]
    Index,
}

pub struct Main {
    link: ComponentLink<Self>,
    access_code: Option<String>,
    task: Option<FetchTask>,
    refresh_task: Option<TimeoutTask>,
    app_failure: bool,
}

#[derive(Debug, Clone)]
pub enum Msg {
    /// Trigger fetching the endpoint information
    FetchEndpoint,
    /// Failed to fetch endpoint information
    FetchBackendFailed,
    /// Trigger an overall application failure
    AppFailure(Toast),
    /// Set the backend information
    Endpoint(BackendInformation),
    /// Exchange the authentication code for an access token
    GetToken,
    /// Set the access token
    SetAccessToken(Token),
    /// Callback when the authentication/login failed
    LoginFailed,
    RetryLogin,
    /// Send to trigger refreshing the access token
    RefreshToken,
    /// Trigger logout
    Logout,
}

impl Component for Main {
    type Message = Msg;
    type Properties = ();
    fn create(_: Self::Properties, link: ComponentLink<Self>) -> Self {
        link.send_message(Msg::FetchEndpoint);

        let location = window().location();
        let url = Url::parse(&location.href().unwrap()).unwrap();

        log::info!("href: {:?}", url);

        let code = url.query_pairs().find_map(|(k, v)| {
            if k == "code" {
                Some(v.to_string())
            } else {
                None
            }
        });

        let error = url.query_pairs().find_map(|(k, v)| {
            if k == "error" {
                Some(v.to_string())
            } else {
                None
            }
        });

        log::info!("Access code: {:?}", code);
        log::info!("Login error: {:?}", error);

        if let Some(error) = error {
            link.send_message(Msg::AppFailure(Toast {
                title: "Failed to log in".into(),
                body: html! {<p>{error}</p>},
                r#type: Type::Danger,
                actions: vec![link.callback(|_| Msg::RetryLogin).into_action("Retry")],
                ..Default::default()
            }));
        }

        // remove code, state and others from the URL bar
        {
            let mut url = url;
            url.query_pairs_mut().clear();
            let url = url.as_str().trim_end_matches('?');
            window()
                .history()
                .unwrap()
                .replace_state_with_url(&JsValue::NULL, "Drogue IoT", Some(url))
                .ok();
        }

        Self {
            link,
            access_code: code,
            task: None,
            refresh_task: None,
            app_failure: false,
        }
    }

    fn update(&mut self, msg: Self::Message) -> ShouldRender {
        match msg {
            Msg::FetchEndpoint => {
                self.task = Some(
                    self.fetch_backend()
                        .expect("Failed to get backend information"),
                );
                true
            }
            Msg::Endpoint(backend) => {
                log::info!("Got backend: {:?}", backend);
                Backend::set(Some(backend));
                self.task = None;
                if !self.app_failure && self.access_code.is_some() {
                    // exchange code for token if we have a code and no app failure
                    log::info!("Exchange access code for token");
                    self.link.send_message(Msg::GetToken);
                }

                true
            }
            Msg::FetchBackendFailed => {
                error(
                    "Failed to fetch backend information",
                    "Could not retrieve information for connecting to the backend.",
                );
                true
            }
            Msg::AppFailure(toast) => {
                ToastDispatcher::default().toast(toast);
                self.app_failure = true;
                true
            }
            Msg::LoginFailed => {
                error("Failed to log in", "Cloud not retrieve access token.");
                self.app_failure = true;
                true
            }
            Msg::RetryLogin => {
                Backend::update_token(None);
                if Backend::reauthenticate().is_err() {
                    error(
                        "Failed to log in",
                        "No backed information present. Unable to trigger login.",
                    );
                }
                false
            }
            Msg::GetToken => {
                // get the access token from the code
                // this can only be called once the backend information and the access code is available
                if Backend::get().is_some() && self.access_code.is_some() {
                    self.task = Some(self.fetch_token().expect("Failed to create request"));
                }
                true
            }
            Msg::SetAccessToken(token) => {
                log::info!("Token: {:?}", token);
                self.task = None;
                Backend::update_token(Some(token.clone()));
                if let Some(timeout) = token.valid_for() {
                    log::info!("Token expires in {:?}", timeout);

                    let mut rem = (timeout.as_secs() as i64) - 30;
                    if rem < 0 {
                        // ensure we are non-negative
                        rem = 0;
                    }

                    if rem < 30 {
                        // refresh now
                        log::info!("Scheduling refresh now (had {} s remaining)", rem);
                        self.link.send_message(Msg::RefreshToken);
                    } else {
                        log::info!("Scheduling refresh in {} seconds", rem);
                        self.refresh_task = Some(TimeoutService::spawn(
                            Duration::from_secs(rem as u64),
                            self.link.callback(|_| {
                                log::info!("Token timer expired, refreshing...");
                                Msg::RefreshToken
                            }),
                        ));
                    }
                } else {
                    log::info!("Token has no expiration set");
                }
                true
            }
            Msg::RefreshToken => {
                log::info!("Refreshing access token");

                match Backend::token().and_then(|t| t.refresh_token) {
                    Some(refresh_token) => {
                        self.task = match self.refresh_token(&refresh_token) {
                            Ok(task) => Some(task),
                            Err(_) => {
                                Backend::reauthenticate().ok();
                                None
                            }
                        }
                    }
                    None => {
                        Backend::reauthenticate().ok();
                    }
                }

                true
            }
            Msg::Logout => {
                Backend::logout().ok();
                false
            }
        }
    }

    fn change(&mut self, _props: Self::Properties) -> ShouldRender {
        false
    }

    fn view(&self) -> Html {
        let sidebar = match Backend::get().is_some() {
            true => html_nested! {
                <PageSidebar>
                    <Nav>
                        <NavList>
                            <NavRouterItem<AppRoute> to=AppRoute::Index>{"Home"}</NavRouterItem<AppRoute>>
                            <NavRouterItem<AppRoute> to=AppRoute::Examples>{"Getting started"}</NavRouterItem<AppRoute>>
                            <NavRouterItem<AppRoute> to=AppRoute::Spy>{"Spy"}</NavRouterItem<AppRoute>>
                        </NavList>
                    </Nav>
                </PageSidebar>
            },
            false => html_nested! {
                <PageSidebar>
                </PageSidebar>
            },
        };

        let tools: Children = Children::new(vec![match Backend::token() {
            Some(token) => {
                let src = token
                    .userinfo
                    .and_then(|user| {
                        if user.email_verified {
                            user.email
                        } else {
                            None
                        }
                    })
                    .map(|email| md5::compute(email.as_bytes()))
                    .map(|hash| format!("https://www.gravatar.com/avatar/{:x}?D=mp", hash))
                    .unwrap_or_else(|| "/images/img_avatar.svg".into());

                html! {
                    <AppLauncher
                        toggle=html!{<Avatar src=src/>}
                        >
                        <AppLauncherItem onclick=self.link.callback(|_|Msg::Logout)>{"Logout"}</AppLauncherItem>
                    </AppLauncher>

                }
            }
            None => html! {},
        }]);

        return html! {
            <>
                <ToastViewer/>

                {
                    if self.is_ready() {

                        html!{
                            <Page
                                logo={html_nested!{
                                    <Logo src="/images/logo.png" alt="Drogue IoT" />
                                }}
                                sidebar=sidebar
                                tools=tools
                                >
                                    <Router<AppRoute, ()>
                                            redirect = Router::redirect(|_|AppRoute::Index)
                                            render = Router::render(|switch: AppRoute| {
                                                match switch {
                                                    AppRoute::Spy => html!{<Spy/>},
                                                    AppRoute::Index => html!{<Index/>},
                                                    AppRoute::Examples => html!{<Examples/>},
                                                }
                                            })
                                        />
                            </Page>
                        }
                    } else if Backend::get().is_some() {
                        html!{ <Placeholder/> }
                    } else {
                        html!{}
                    }
                }

            </>
        };
    }
}

impl Main {
    /// Check if the app and backend are ready to show the application.
    fn is_ready(&self) -> bool {
        !self.app_failure && Backend::get().is_some() && Backend::access_token().is_some()
    }

    fn fetch_backend(&self) -> Result<FetchTask, anyhow::Error> {
        let req = Request::get("/endpoints/backend.json").body(Nothing)?;

        let opts = FetchOptions {
            cache: Some(Cache::NoCache),
            ..Default::default()
        };

        FetchService::fetch_with_options(
            req,
            opts,
            self.link.callback(
                |response: Response<Json<Result<BackendInformation, Error>>>| {
                    log::info!("Backend: {:?}", response);
                    if let (meta, Json(Ok(body))) = response.into_parts() {
                        if meta.status.is_success() {
                            return Msg::Endpoint(body);
                        }
                    }
                    Msg::FetchBackendFailed
                },
            ),
        )
    }

    fn refresh_token(&self, refresh_token: &str) -> Result<FetchTask, anyhow::Error> {
        let mut url = Backend::url("/ui/refresh")
            .ok_or_else(|| anyhow::anyhow!("Missing backend information"))?;

        url.query_pairs_mut()
            .append_pair("refresh_token", refresh_token);

        let req = Request::get(url.to_string()).body(Nothing)?;

        let opts = FetchOptions {
            cache: Some(Cache::NoCache),
            ..Default::default()
        };

        FetchService::fetch_with_options(
            req,
            opts,
            self.link.callback(
                |response: Response<Json<Result<serde_json::Value, Error>>>| {
                    log::info!("Response from refreshing token: {:?}", response);
                    Self::from_response(response)
                },
            ),
        )
    }

    fn fetch_token(&self) -> Result<FetchTask, anyhow::Error> {
        let mut url = Backend::url("/ui/token")
            .ok_or_else(|| anyhow::anyhow!("Missing backend information"))?;

        url.query_pairs_mut().append_pair(
            "code",
            &self
                .access_code
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Missing access code"))?,
        );

        let req = Request::get(url.to_string()).body(Nothing)?;

        let opts = FetchOptions {
            cache: Some(Cache::NoCache),
            ..Default::default()
        };

        FetchService::fetch_with_options(
            req,
            opts,
            self.link.callback(
                |response: Response<Json<Result<serde_json::Value, Error>>>| {
                    log::info!("Code to token response: {:?}", response);
                    Self::from_response(response)
                },
            ),
        )
    }

    fn from_response(response: Response<Json<Result<serde_json::Value, Error>>>) -> Msg {
        if let (meta, Json(Ok(value))) = response.into_parts() {
            if meta.status.is_success() {
                let access_token = value["bearer"]["access_token"]
                    .as_str()
                    .map(|s| s.to_string());
                let refresh_token = value["bearer"]["refresh_token"]
                    .as_str()
                    .map(|s| s.to_string());
                let id_token = value["bearer"]["id_token"].as_str().map(|s| s.to_string());

                let userinfo: Option<UserInfo> =
                    serde_json::from_value(value["userinfo"].clone()).unwrap_or_default();

                let expires = match value["expires"].as_str() {
                    Some(expires) => DateTime::parse_from_rfc3339(expires).ok(),
                    None => None,
                }
                .map(|expires| expires.with_timezone(&Utc));

                let token = match (access_token, id_token) {
                    (Some(access_token), Some(id_token)) => Some(Token {
                        access_token,
                        refresh_token,
                        id_token,
                        expires,
                        userinfo,
                    }),
                    _ => None,
                };

                log::info!("Token: {:?}", token);

                match token {
                    Some(token) => Msg::SetAccessToken(token),
                    None => Msg::LoginFailed,
                }
            } else {
                Msg::LoginFailed
            }
        } else {
            Msg::LoginFailed
        }
    }
}
