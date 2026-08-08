//! Supervisor lifecycle tests. nextest runs each test in its own process, so
//! every test owns the process-global supervisor — but each still uses its
//! own `TestRoot` dir, and tests that need a *second* library (destroy →
//! fresh) just keep using the same root after wiping it.

use std::time::Duration;

use halogen_api::ApiClient;
use halogen_wire::{LoginData, UserStoreData};
use url::Url;

use crate::{EmbeddedDirs, EmbeddedStatus};

fn init_test_tracing() {
    use std::sync::OnceLock;
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        use tracing_subscriber::EnvFilter;
        let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,hyper=warn,hyper_util=warn,sqlx=warn,sea_orm=warn,reqwest=warn")
        });
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_test_writer()
            .try_init();
    });
}

fn dirs_in(root: &halogen_fixture::test_support::TestRoot) -> EmbeddedDirs {
    EmbeddedDirs::new(root.path().join("server"))
}

async fn boot(dirs: &EmbeddedDirs) -> (String, ApiClient) {
    let base_url = crate::ensure_started(dirs.clone()).expect("ensure_started");
    let port = crate::wait_ready().await.expect("embedded server ready");
    assert!(base_url.ends_with(&format!(":{port}")), "url/port agree");
    let client = ApiClient::new(Url::parse(&base_url).expect("parse base url"));
    (base_url, client)
}

async fn login(client: &ApiClient, dirs: &EmbeddedDirs) -> String {
    let creds = crate::credentials(dirs).expect("credentials provisioned");
    let token = client
        .login(LoginData {
            username: creds.username.clone(),
            password: creds.password.clone(),
        })
        .await
        .expect("login with generated credentials")
        .token;
    client.set_token(Some(token.clone()));
    token
}

/// First boot provisions everything (dirs, secrets 0600, admin, queue
/// playlist), serves /healthz, and the generated credentials log in.
#[tokio::test(flavor = "multi_thread")]
async fn first_boot_provisions_and_serves() {
    init_test_tracing();
    let mut root = halogen_fixture::test_support::TestRoot::new("embedded_first_boot");
    let dirs = dirs_in(&root);
    assert!(!dirs.library_exists());

    let (_base, client) = boot(&dirs).await;
    client.health().await.expect("healthz answers");

    assert!(dirs.library_exists(), "secrets provisioned");
    let token = login(&client, &dirs).await;
    assert!(!token.is_empty());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dirs.root().join("secrets.json"))
            .expect("secrets metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "secrets.json is 0600");
    }

    crate::stop().await;
    root.mark_success();
}

/// stop() then ensure_started() again: same port, same signing secret — a
/// token minted before the stop still authenticates after the restart.
#[tokio::test(flavor = "multi_thread")]
async fn stop_start_keeps_port_and_sessions() {
    init_test_tracing();
    let mut root = halogen_fixture::test_support::TestRoot::new("embedded_stop_start");
    let dirs = dirs_in(&root);

    let (base1, client) = boot(&dirs).await;
    let _token = login(&client, &dirs).await;

    crate::stop().await;
    assert_eq!(crate::status(), EmbeddedStatus::Stopped);

    let (base2, _client2) = boot(&dirs).await;
    assert_eq!(base1, base2, "port is sticky across stop/start");

    // The client still holds the pre-stop token — an authed route must work,
    // proving the signing secret survived (`GET /admin/*` needs admin; the
    // ws-ticket mint is the simplest authed call).
    client
        .mint_ws_ticket()
        .await
        .expect("pre-stop token still valid");

    crate::stop().await;
    root.mark_success();
}

/// Every `ensure_started` — including ones landing mid-startup, exactly when
/// the manager is taking the staged socket — must return the ONE port the run
/// will serve on. Regression test for the start-turnaround race: the take used
/// to empty the staging slot before `Starting` was published, so a caller in
/// that window re-bound (sticky port still held → ephemeral fallback) and got
/// a URL nothing would ever serve. The config-load resolver calls
/// `ensure_started` on every load, so this fired in real use.
#[tokio::test(flavor = "multi_thread")]
async fn ensure_started_port_stable_through_startup() {
    init_test_tracing();
    let mut root = halogen_fixture::test_support::TestRoot::new("embedded_port_stable");
    let dirs = dirs_in(&root);

    // Hammer ensure_started while the first start is in flight — with a beat
    // after the first poke so the manager is GUARANTEED to have taken the
    // staged socket mid-loop (otherwise the whole loop can finish before it is
    // ever scheduled and the interesting window goes untested).
    let mut urls = vec![crate::ensure_started(dirs.clone()).expect("ensure_started")];
    tokio::time::sleep(Duration::from_millis(50)).await;
    for _ in 0..200 {
        urls.push(crate::ensure_started(dirs.clone()).expect("ensure_started"));
    }
    let port = crate::wait_ready().await.expect("embedded server ready");
    let expected = format!("http://127.0.0.1:{port}");
    assert!(
        urls.iter().all(|u| *u == expected),
        "every ensure_started URL must be the served port {port}; got {:?}",
        urls.iter().collect::<std::collections::HashSet<_>>()
    );

    // And once Running, further calls keep answering the live port.
    let again = crate::ensure_started(dirs.clone()).expect("ensure_started while running");
    assert_eq!(again, expected);

    crate::stop().await;
    root.mark_success();
}

/// The admin restart endpoint drains and re-serves in-process on the same
/// port — the embedded replacement for the binary's execv.
#[tokio::test(flavor = "multi_thread")]
async fn api_restart_reserves_in_process() {
    init_test_tracing();
    let mut root = halogen_fixture::test_support::TestRoot::new("embedded_api_restart");
    let dirs = dirs_in(&root);

    let (base1, client) = boot(&dirs).await;
    login(&client, &dirs).await;

    // Subscribe BEFORE requesting the restart: the supervisor publishes no
    // status change until the old run has fully drained (Starting comes after
    // serve returns), so the first wakeup below proves the drain happened —
    // reading `status()` directly would race against the still-Running old
    // value.
    let mut rx = crate::subscribe_status().expect("status watch");
    rx.mark_unchanged();
    client.restart_server().await.expect("restart accepted");
    let port = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            rx.changed().await.expect("supervisor alive");
            if let EmbeddedStatus::Running { port } = *rx.borrow_and_update() {
                return port;
            }
        }
    })
    .await
    .expect("running again after restart");
    assert_eq!(
        base1,
        format!("http://127.0.0.1:{port}"),
        "same port after restart"
    );
    client.health().await.expect("healthz after restart");

    crate::stop().await;
    root.mark_success();
}

/// recover_admin rotates the password (DB + secrets) so a drifted secrets
/// file heals: old credentials stop working, recovered ones log in.
#[tokio::test(flavor = "multi_thread")]
async fn recover_admin_heals_credentials() {
    init_test_tracing();
    let mut root = halogen_fixture::test_support::TestRoot::new("embedded_recover");
    let dirs = dirs_in(&root);

    let (_base, client) = boot(&dirs).await;
    let stale = crate::credentials(&dirs).expect("initial credentials");
    login(&client, &dirs).await;

    crate::stop().await;
    let recovered = crate::recover_admin(&dirs).await.expect("recover admin");
    assert_eq!(recovered.username, stale.username);
    assert_ne!(recovered.password, stale.password, "password rotated");

    let (_base, client) = boot(&dirs).await;
    client
        .login(LoginData {
            username: stale.username.clone(),
            password: stale.password.clone(),
        })
        .await
        .expect_err("stale password rejected");
    login(&client, &dirs).await; // recovered credentials from disk

    crate::stop().await;
    root.mark_success();
}

/// destroy() wipes the library; the next start provisions a fresh one with
/// new credentials.
#[tokio::test(flavor = "multi_thread")]
async fn destroy_wipes_and_reprovisions() {
    init_test_tracing();
    let mut root = halogen_fixture::test_support::TestRoot::new("embedded_destroy");
    let dirs = dirs_in(&root);

    let (_base, client) = boot(&dirs).await;
    let first = crate::credentials(&dirs).expect("first credentials");
    login(&client, &dirs).await;

    crate::destroy(&dirs).await.expect("destroy");
    assert!(!dirs.root().exists(), "root removed");
    assert_eq!(crate::status(), EmbeddedStatus::Stopped);

    let (_base, client) = boot(&dirs).await;
    let second = crate::credentials(&dirs).expect("fresh credentials");
    assert_ne!(
        first.password, second.password,
        "fresh library, fresh secrets"
    );
    login(&client, &dirs).await;

    crate::stop().await;
    root.mark_success();
}

/// Multi-user: `remember_new_user` + the admin create endpoint + per-user
/// silent login, and `recover_user` re-keying a drifted password — the full
/// story behind the embedded "Add account" flow.
#[tokio::test(flavor = "multi_thread")]
async fn second_user_provisioning_and_recovery() {
    init_test_tracing();
    let mut root = halogen_fixture::test_support::TestRoot::new("embedded_second_user");
    let dirs = dirs_in(&root);

    let (base, admin_client) = boot(&dirs).await;
    login(&admin_client, &dirs).await;

    // Provision: generate (no persist), create server-side with exactly those
    // credentials, THEN persist — a rejected create must never touch
    // secrets.json. Usernames lowercase.
    let creds = crate::generate_credentials("Buddy");
    assert_eq!(creds.username, "buddy");
    assert!(
        !crate::has_credentials(&dirs, "buddy").expect("secrets readable"),
        "nothing persisted before the server-side create"
    );
    admin_client
        .create_user(UserStoreData {
            username: creds.username.clone(),
            password: creds.password.clone(),
            password_confirm: creds.password.clone(),
            is_admin: Some(true),
        })
        .await
        .expect("admin creates the user");
    crate::remember_user(&dirs, &creds).expect("persist after create");
    assert!(crate::has_credentials(&dirs, "buddy").expect("secrets readable"));

    // Silent login as the new user from the stored secrets.
    let stored = crate::credentials_for(&dirs, "buddy").expect("stored credentials");
    assert_eq!(stored.password, creds.password);
    let user_client = ApiClient::new(Url::parse(&base).expect("base url"));
    user_client
        .login(LoginData {
            username: stored.username.clone(),
            password: stored.password.clone(),
        })
        .await
        .expect("second user's silent login");

    // Drift: recovery rotates the DB + secrets against the RUNNING server.
    let rotated = crate::recover_user(&dirs, "buddy")
        .await
        .expect("recover user");
    assert_ne!(rotated.password, creds.password, "password rotated");
    user_client
        .login(LoginData {
            username: "buddy".to_string(),
            password: creds.password.clone(),
        })
        .await
        .expect_err("stale password rejected");
    user_client
        .login(LoginData {
            username: rotated.username.clone(),
            password: rotated.password.clone(),
        })
        .await
        .expect("rotated password logs in");

    crate::stop().await;
    root.mark_success();
}

/// Start pokes issued WHILE the server runs (every embedded config load /
/// auth pass sends one) must not survive a `stop()`: pre-fix they queued
/// behind the blocked manager and rebooted the server the moment the stopped
/// run drained — a deliberate stop silently undone.
#[tokio::test(flavor = "multi_thread")]
async fn stale_start_pokes_do_not_resurrect_a_stopped_server() {
    init_test_tracing();
    let mut root = halogen_fixture::test_support::TestRoot::new("embedded_stale_pokes");
    let dirs = dirs_in(&root);

    let (_base, client) = boot(&dirs).await;
    client.health().await.expect("healthz answers");

    // Queue several pokes while the run is live (the manager is blocked inside
    // the run, so these sit in the channel).
    for _ in 0..5 {
        crate::ensure_started(dirs.clone()).expect("poke while running");
    }

    crate::stop().await;
    assert!(
        matches!(crate::status(), EmbeddedStatus::Stopped),
        "stop() resolves to Stopped"
    );

    // Give the manager ample time to (wrongly) consume the queued pokes and
    // reboot; the status must remain Stopped.
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let status = crate::status();
        assert!(
            matches!(status, EmbeddedStatus::Stopped),
            "server resurrected after stop(): {status:?}"
        );
    }

    // A GENUINE post-stop start must still work.
    let (_base, client) = boot(&dirs).await;
    client.health().await.expect("healthz after restart");

    crate::stop().await;
    root.mark_success();
}
