use twitch_api::client::CompatError;
use twitch_api::twitch_oauth2::tokens::errors::AppAccessTokenError;
use twitch_api::twitch_oauth2::{AppAccessToken, ClientId, ClientSecret};
use twitch_api::HelixClient;

pub struct TwitchClientBundle {
    #[allow(unused)]
    client_id: ClientId,
    #[allow(unused)]
    client_secret: ClientSecret,
    app_token: AppAccessToken,
    twitch_client: HelixClient<'static, reqwest::Client>,
}

impl TwitchClientBundle {
    pub async fn new(
        client_id: ClientId,
        client_secret: ClientSecret,
    ) -> Result<Self, AppAccessTokenError<CompatError<reqwest::Error>>> {
        let twitch_client = HelixClient::<reqwest::Client>::new();
        let app_token = AppAccessToken::get_app_access_token(
            &twitch_client,
            client_id.clone(),
            client_secret.clone(),
            vec![],
        )
        .await?;
        Ok(Self {
            twitch_client,
            client_id,
            client_secret,
            app_token,
        })
    }
    pub async fn req_get<R, D>(
        &self,
        request: R,
    ) -> Result<
        twitch_api::helix::Response<R, D>,
        twitch_api::helix::ClientRequestError<reqwest::Error>,
    >
    where
        R: twitch_api::helix::Request<Response = D>
            + twitch_api::helix::Request
            + twitch_api::helix::RequestGet,
        D: serde::de::DeserializeOwned + PartialEq,
    {
        self.twitch_client.req_get(request, &self.app_token).await
    }
}
