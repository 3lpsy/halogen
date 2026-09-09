//! Discover searches, remote previews, and subscription use local directory/feed fixtures only.

use halogen_e2e::{
    body_text, browser_session, login_via_ui, require_dist, run_session, shot, wait_for_text,
};
use halogen_integ::{
    api,
    support::{SpawnOptions, spawn_with},
};
use serde_json::json;
use std::time::Duration;
use thirtyfour::{components::SelectElement, prelude::*};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{path, query_param},
};

#[tokio::test]
#[ignore = "needs Chrome + chromedriver + a built dist/ (run via `just test-e2e`)"]
async fn discover_episode_search_and_subscribe() {
    if !require_dist() {
        return;
    }
    let provider = MockServer::start().await;
    let feed = format!("{}/feed/0", provider.uri());
    let description = "A complete podcast description from the remote RSS feed.";
    Mock::given(path("/search"))
        .and(query_param("entity", "podcast"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"results":[{"collectionName":"Show 0","feedUrl":feed}]})),
        )
        .expect(1)
        .mount(&provider)
        .await;
    let episodes: Vec<_> = (0..30).map(|i| json!({
        "kind":"podcast-episode", "trackId":i, "trackName":format!("Meditation episode {i}"),
        "collectionName":format!("Show {}", i % 3), "feedUrl":format!("{}/feed/{}", provider.uri(), i % 3),
        "episodeGuid":format!("episode-{i}"), "description":format!("Episode synopsis {i}"),
        "releaseDate":"2026-09-08T00:00:00Z", "trackTimeMillis":120000
    })).collect();
    Mock::given(path("/search"))
        .and(query_param("entity", "podcastEpisode"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"results":episodes})))
        .expect(1)
        .mount(&provider)
        .await;
    Mock::given(path("/gpodder"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(1)
        .mount(&provider)
        .await;
    let rss = format!(
        r#"<rss version="2.0"><channel><title>Show 0</title><link>{feed}</link><description>{description}</description><item><title>Meditation episode 0</title><guid>episode-0</guid><description>Episode synopsis 0</description><enclosure url="{}/audio" type="audio/mpeg" length="8"/></item></channel></rss>"#,
        provider.uri()
    );
    Mock::given(path("/feed/0"))
        .respond_with(ResponseTemplate::new(200).set_body_string(rss))
        .mount(&provider)
        .await;
    Mock::given(path("/audio"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"fixture".to_vec()))
        .mount(&provider)
        .await;
    let app = spawn_with(SpawnOptions {
        discover_itunes_base_url: Some(format!("{}/search", provider.uri())),
        discover_gpodder_base_url: Some(format!("{}/gpodder", provider.uri())),
        ..Default::default()
    })
    .await;
    let admin = app.seed_admin().await;
    let Some((_guard, driver)) = browser_session().await else {
        return;
    };
    run_session(driver, "discover episodes journey", async |driver| {
        login_via_ui(&driver, &app.base_url, &admin.username, &admin.password).await;
        driver.goto(format!("{}/discover", app.base_url)).await?;
        let selector = driver
            .query(By::Css("select[aria-label='Search by']"))
            .first()
            .await?;
        assert_eq!(selector.value().await?.as_deref(), Some("podcast"));
        let search_box = driver.find(By::Css("input[type='search']")).await?;
        assert!(
            search_box.rect().await?.width >= 120.0,
            "search field remains usable beside the mode dropdown"
        );
        assert!(
            selector.rect().await?.width <= 180.0,
            "mode dropdown stays compact"
        );
        driver
            .find(By::Css("input[type='search']"))
            .await?
            .send_keys("Meditation")
            .await?;
        driver
            .query(By::Css("button[type='submit']"))
            .and_enabled()
            .first()
            .await?
            .click()
            .await?;
        assert!(wait_for_text(&driver, "Show 0", Duration::from_secs(15)).await);
        shot(&driver, "01-podcast-results").await;
        driver
            .find(By::Css("a[href^='/discover/podcasts/']"))
            .await?
            .click()
            .await?;
        assert!(wait_for_text(&driver, description, Duration::from_secs(15)).await);
        shot(&driver, "04-discover-podcast").await;
        assert!(
            driver
                .current_url()
                .await?
                .path()
                .starts_with("/discover/podcasts/")
        );
        driver.back().await?;
        let selector = driver
            .query(By::Css("select[aria-label='Search by']"))
            .first()
            .await?;
        SelectElement::new(&selector)
            .await?
            .select_by_value("episode")
            .await?;
        driver
            .query(By::Css("button[type='submit']"))
            .and_enabled()
            .first()
            .await?
            .click()
            .await?;
        assert!(wait_for_text(&driver, "Meditation episode 0", Duration::from_secs(15)).await);
        assert!(body_text(&driver).await.contains("Show 1"));
        shot(&driver, "02-cross-podcast-episodes").await;
        driver
            .find(By::Id("discover-sentinel"))
            .await?
            .scroll_into_view()
            .await?;
        assert!(wait_for_text(&driver, "Meditation episode 29", Duration::from_secs(15)).await);
        let first = driver
            .find(By::Css("a[href^='/discover/episodes/']"))
            .await?;
        first.scroll_into_view().await?;
        first.click().await?;
        assert!(wait_for_text(&driver, "Episode synopsis 0", Duration::from_secs(10)).await);
        shot(&driver, "03-discover-episode").await;
        assert!(
            driver
                .current_url()
                .await?
                .path()
                .starts_with("/discover/episodes/")
        );
        driver
            .find(By::XPath("//button[normalize-space(.)='Show 0']"))
            .await?
            .click()
            .await?;
        assert!(wait_for_text(&driver, description, Duration::from_secs(15)).await);
        driver
            .query(By::XPath("//button[normalize-space(.)='Subscribe']"))
            .and_enabled()
            .first()
            .await?
            .click()
            .await?;
        let dialog = driver
            .query(By::Css(
                "[role='dialog'][aria-labelledby='discover-subscribe-title']",
            ))
            .and_displayed()
            .first()
            .await?;
        assert!(dialog.text().await?.contains("Show 0"));
        dialog
            .find(By::Css("button.btn-primary"))
            .await?
            .click()
            .await?;
        assert!(wait_for_text(&driver, "Show 0", Duration::from_secs(20)).await);
        assert_eq!(driver.current_url().await?.path(), "/podcasts");
        let client = api(&app, &admin.token);
        let saved = tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                let data = client.list_podcasts(Default::default()).await.unwrap();
                if let Some(podcast) = data.data.into_iter().find(|p| p.feed_url == feed) {
                    break podcast;
                }
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        })
        .await
        .expect("subscription reaches the server");
        assert_eq!(saved.title, "Show 0");
        assert_eq!(saved.description, description);
        Ok::<_, WebDriverError>(())
    })
    .await;
}
