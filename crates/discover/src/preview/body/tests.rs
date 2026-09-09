use super::*;
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn completes_preview_without_waiting_for_feed_tail() {
    let (mut sender, receiver) = tokio::io::duplex(4096);
    let producer = tokio::spawn(async move {
        sender.write_all(b"<rss version=\"2.0\"><channel><title>Show</title><description>About</description>").await.unwrap();
        for id in 0..MAX_EPISODES {
            let item = format!("<item><title>Episode {id}</title><guid>{id}</guid></item>");
            sender.write_all(item.as_bytes()).await.unwrap();
        }
        // Keep the stream open: a preview must not wait for the remaining archive.
        std::future::pending::<()>().await;
    });
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        read_prefix(tokio::io::BufReader::new(receiver)),
    )
    .await;
    producer.abort();
    let body = result.expect("preview must finish before EOF").unwrap();
    let channel = rss::Channel::read_from(body.as_slice()).unwrap();
    assert_eq!(channel.description(), "About");
    assert_eq!(channel.items().len(), MAX_EPISODES);
}

#[tokio::test]
async fn malformed_and_excessively_nested_feeds_are_rejected() {
    for xml in [
        "<rss><channel><title>unfinished".to_string(),
        "<!DOCTYPE rss><rss><channel/></rss>".to_string(),
        format!("<rss><channel>{}", "<nested>".repeat(MAX_DEPTH)),
    ] {
        assert!(read_prefix(xml.as_bytes()).await.is_err());
    }
}
