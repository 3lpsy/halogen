use halogen_discover::DiscoverService;
use halogen_wire::DiscoverProvider;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

// Separate integration process isolates the global network policy from provider mocks.
#[tokio::test]
async fn private_initial_urls_and_redirects_are_rejected() {
    halogen_net::configure(false);
    let service = DiscoverService::with_bases(String::new(), String::new());
    for url in [
        "http://127.0.0.1/feed",
        "http://[::1]/feed",
        "http://169.254.169.254/feed",
    ] {
        let error = service
            .podcast_preview(url, DiscoverProvider::Itunes)
            .await
            .unwrap_err();
        assert!(error.contains("Non-public"), "{error}");
    }
    let server = MockServer::start().await;
    Mock::given(path("/redirect"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("Location", format!("{}/target", server.uri())),
        )
        .mount(&server)
        .await;
    Mock::given(path("/target"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let client = reqwest::Client::builder()
        .redirect(halogen_net::guarded_redirect_policy())
        .build()
        .unwrap();
    let error = client
        .get(format!("{}/redirect", server.uri()))
        .send()
        .await
        .unwrap_err();
    assert!(error.is_redirect());
}
