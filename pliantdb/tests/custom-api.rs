//! Tests a single server with multiple simultaneous connections.

use pliantdb::{
    client::{url::Url, Client},
    core::{
        permissions::{
            ActionNameList, Actionable, Dispatcher, Permissions, ResourceName, Statement,
        },
        test_util::{Basic, TestDirectory},
    },
    server::{Configuration, Server},
};
use pliantdb_core::backend::{Backend, CustomApi};
use serde::{Deserialize, Serialize};

#[derive(Debug, Dispatcher)]
#[dispatcher(input = CustomRequest)]
struct CustomBackend;

impl Backend for CustomBackend {
    type CustomApi = Self;
}

impl CustomApi for CustomBackend {
    type Request = CustomRequest;

    type Response = CustomResponse;

    type Dispatcher = Self;
}

#[derive(Serialize, Deserialize, Debug, Actionable)]
enum CustomRequest {
    #[actionable(protection = "none")]
    Ping,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
enum CustomResponse {
    Pong,
}

#[tokio::test]
async fn custom_api() -> anyhow::Result<()> {
    let dir = TestDirectory::new("custom_api.pliantdb");
    let server = Server::<CustomBackend>::open(
        dir.as_ref(),
        Configuration {
            default_permissions: Permissions::from(vec![Statement {
                resources: vec![ResourceName::any()],
                actions: ActionNameList::All,
                allowed: true,
            }]),
            ..Configuration::default_with_dispatcher(CustomBackend)
        },
    )
    .await?;
    server
        .install_self_signed_certificate("test", false)
        .await?;
    let certificate = server.certificate().await?;
    server.register_schema::<Basic>().await?;
    tokio::spawn(async move { server.listen_on(12346).await });

    let client =
        Client::<CustomBackend>::new(Url::parse("pliantdb://localhost:12346")?, Some(certificate))
            .await?;

    let CustomResponse::Pong = client.send_api_request(CustomRequest::Ping).await?;

    Ok(())
}

impl CustomRequestDispatcher for CustomBackend {
    type Output = CustomResponse;

    type Error = anyhow::Error;

    type PingHandler = Self;
}

#[actionable::async_trait]
impl PingHandler for CustomBackend {
    type Dispatcher = Self;

    async fn handle(
        _dispatcher: &Self::Dispatcher,
        _permissions: &Permissions,
    ) -> Result<CustomResponse, anyhow::Error> {
        Ok(CustomResponse::Pong)
    }
}
