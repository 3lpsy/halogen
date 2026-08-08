#[cfg(test)]
mod config_tests {
    use std::env;
    use std::fs;
    use std::io;
    use std::path::PathBuf;
    use std::process;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use clap::Parser;

    use super::super::*;

    fn make_temp_config(content: &str) -> PathBuf {
        // PID + counter so each test process gets a unique file: under nextest
        // (process per test) the counter resets to 0 in every process, so without
        // the PID concurrent tests would clobber/delete a shared `_0.toml`.
        static CONFIG_COUNTER: AtomicUsize = AtomicUsize::new(0);
        let counter = CONFIG_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = env::temp_dir().join(format!(
            "halogen_test_config_{}_{counter}.toml",
            process::id()
        ));
        let mut f = fs::File::create(&path).unwrap();
        io::Write::write_all(&mut f, content.as_bytes()).unwrap();
        drop(f);
        path
    }

    fn test_key() -> String {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        format!("{}", COUNTER.fetch_add(1, Ordering::SeqCst))
    }

    #[test]
    fn test_defaults_resolve() {
        let key = test_key();
        set_test_env_key(&key);
        let cli = Cli::parse_from(["halogen-server"]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.listen_address, "127.0.0.1");
        assert_eq!(cfg.listen_port, 8080);
        assert!(!cfg.server_disable_polling_service);
        assert_eq!(cfg.db_path, PathBuf::from("halogen.db"));
        assert_eq!(cfg.media_root, PathBuf::from("./media"));
        assert_eq!(
            cfg.subscription_fallback_poll_interval,
            Duration::from_secs(3600)
        );
        assert_eq!(cfg.subscription_fallback_max_episodes, 50);
        assert_eq!(cfg.subscription_max_concurrent_downloads, 3);
        assert_eq!(
            cfg.subscription_download_stuck_after,
            Duration::from_secs(6 * 60 * 60)
        );
        assert_eq!(cfg.subscription_download_max_attempts, 10);
        assert_eq!(cfg.auth_token_expiry_minutes, 60 * 24 * 7);
        assert!(!cfg.auth_token_secret.is_empty());
        assert_eq!(cfg.log_level, "info");
        assert!(cfg.log_file.is_none());
        assert!(!cfg.log_target);
        assert!(cfg.log_line_number);
        assert!(cfg.log_file_name);
        assert!(!cfg.log_target);
    }

    #[test]
    fn test_config_file_overrides_defaults() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config(
            "[server]\nlisten_port = 9090\nlisten_address = \"0.0.0.0\"\n\n[db]\npath = \"/tmp/halogen.toml_test.db\"\n\n[subscription]\nfallback_max_episodes = 100\n",
        );
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.listen_address, "0.0.0.0");
        assert_eq!(cfg.listen_port, 9090);
        assert_eq!(cfg.db_path, PathBuf::from("/tmp/halogen.toml_test.db"));
        assert_eq!(cfg.subscription_fallback_max_episodes, 100);
        assert_eq!(
            cfg.subscription_fallback_poll_interval,
            Duration::from_secs(3600)
        );
        fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_env_overrides_config_file() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path =
            make_temp_config("[server]\nlisten_port = 8080\nlisten_address = \"127.0.0.1\"\n");
        unsafe { env::set_var(format!("HALOGEN_LISTEN_ADDRESS_{key}"), "192.168.1.1") };
        unsafe {
            env::set_var(
                format!("HALOGEN_SUBSCRIPTION_FALLBACK_MAX_EPISODES_{key}"),
                "200",
            )
        };
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.listen_address, "192.168.1.1");
        assert_eq!(cfg.listen_port, 8080);
        assert_eq!(cfg.subscription_fallback_max_episodes, 200);
        fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_cli_overrides_env_and_config() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path =
            make_temp_config("[server]\nlisten_port = 7070\nlisten_address = \"127.0.0.1\"\n");
        unsafe { env::set_var(format!("HALOGEN_LISTEN_ADDRESS_{key}"), "192.168.1.1") };
        let cli = Cli::parse_from([
            "halogen-server",
            "-c",
            config_path.to_str().unwrap(),
            "--listen-address",
            "10.0.0.1",
            "--listen-port",
            "3000",
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.listen_address, "10.0.0.1");
        assert_eq!(cfg.listen_port, 3000);
        fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_full_priority_chain() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config(
            "[server]\nlisten_port = 8080\n\n[subscription]\nfallback_max_episodes = 50\n",
        );
        unsafe { env::set_var(format!("HALOGEN_LISTEN_PORT_{key}"), "9999") };
        let cli = Cli::parse_from([
            "halogen-server",
            "-c",
            config_path.to_str().unwrap(),
            "--listen-address",
            "from-cli",
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.listen_address, "from-cli");
        assert_eq!(cfg.listen_port, 9999);
        assert_eq!(cfg.subscription_fallback_max_episodes, 50);
        fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_all_three_sources_together() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config(
            "[server]\nlisten_address = \"from-toml\"\nlisten_port = 9090\n\n[db]\npath = \"/tmp/toml.db\"\n\n[media]\nroot = \"/tmp/toml-media\"\n\n[subscription]\nfallback_max_episodes = 50\nfallback_poll_interval = 3600\n\n[log]\nlevel = \"warn\"\ntarget = true\nfile_name = true\n",
        );
        unsafe { env::set_var(format!("HALOGEN_LISTEN_ADDRESS_{key}"), "from-env") };
        unsafe { env::set_var(format!("HALOGEN_DB_PATH_{key}"), "/tmp/env.db") };
        unsafe {
            env::set_var(
                format!("HALOGEN_SUBSCRIPTION_FALLBACK_MAX_EPISODES_{key}"),
                "200",
            )
        };
        unsafe { env::set_var(format!("HALOGEN_LOG_FILE_{key}"), "/tmp/env.log") };
        unsafe { env::set_var(format!("HALOGEN_LOG_LEVEL_{key}"), "error") };
        unsafe { env::set_var(format!("HALOGEN_LOG_LINE_NUMBER_{key}"), "false") };
        let cli = Cli::parse_from([
            "halogen-server",
            "-c",
            config_path.to_str().unwrap(),
            "--media-root",
            "/tmp/cli-media",
            "--log-line-number",
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.media_root, PathBuf::from("/tmp/cli-media"));
        assert!(cfg.log_line_number);
        assert_eq!(cfg.listen_address, "from-env");
        assert_eq!(cfg.db_path, PathBuf::from("/tmp/env.db"));
        assert_eq!(cfg.subscription_fallback_max_episodes, 200);
        assert_eq!(cfg.log_level, "error");
        assert_eq!(cfg.log_file, Some(PathBuf::from("/tmp/env.log")));
        assert_eq!(cfg.listen_port, 9090);
        assert_eq!(
            cfg.subscription_fallback_poll_interval,
            Duration::from_secs(3600)
        );
        assert!(cfg.log_target);
        fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_invalid_config_file_rejected() {
        let key = test_key();
        set_test_env_key(&key);
        let tmp = env::temp_dir().join(format!("halogen_bad_config_{}.toml", process::id()));
        let cli = Cli::parse_from(["halogen-server", "-c", tmp.to_str().unwrap()]);
        let result = Config::resolve(&cli);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_env_port_parsing() {
        let key = test_key();
        set_test_env_key(&key);
        unsafe { env::set_var(format!("HALOGEN_LISTEN_PORT_{key}"), "5432") };
        let cli = Cli::parse_from(["halogen-server"]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.listen_port, 5432);
    }

    #[test]
    fn test_env_log_level() {
        let key = test_key();
        set_test_env_key(&key);
        unsafe { env::set_var(format!("HALOGEN_LOG_LEVEL_{key}"), "debug") };
        let cli = Cli::parse_from(["halogen-server"]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.log_level, "debug");
    }

    /// `server_fetch_user_agent` resolves through all three layers and defaults
    /// to `None`, which is what makes each outbound client fall back to its own
    /// purpose-tagged UA (see `halogen_net::user_agent`).
    #[test]
    fn test_fetch_user_agent_defaults_to_none() {
        let key = test_key();
        set_test_env_key(&key);
        let cli = Cli::parse_from(["halogen-server"]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.server_fetch_user_agent, None);
    }

    #[test]
    fn test_fetch_user_agent_from_toml_env_and_cli() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config("[server]\nfetch_user_agent = \"from-toml/1.0\"\n");

        // TOML alone.
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(
            cfg.server_fetch_user_agent.as_deref(),
            Some("from-toml/1.0")
        );

        // env beats TOML.
        unsafe {
            env::set_var(
                format!("HALOGEN_SERVER_FETCH_USER_AGENT_{key}"),
                "from-env/1.0",
            )
        };
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.server_fetch_user_agent.as_deref(), Some("from-env/1.0"));

        // CLI beats both.
        let cli = Cli::parse_from([
            "halogen-server",
            "-c",
            config_path.to_str().unwrap(),
            "--server-fetch-user-agent",
            "from-cli/1.0",
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.server_fetch_user_agent.as_deref(), Some("from-cli/1.0"));
        fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_display_trait() {
        let key = test_key();
        set_test_env_key(&key);
        let cli = Cli::parse_from(["halogen-server"]);
        let cfg = Config::resolve(&cli).unwrap();
        let display = format!("{}", cfg);
        assert!(display.contains("listen_address"));
        assert!(display.contains("listen_port"));
        assert!(display.contains("server_disable_polling_service"));
        assert!(display.contains("8080"));
    }

    #[test]
    fn test_no_sync_before_default_and_toml_override() {
        let key = test_key();
        set_test_env_key(&key);

        // Default is 2026-01-01.
        let cli = Cli::parse_from(["halogen-server"]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(
            cfg.subscription_no_sync_before,
            chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()
        );

        // TOML overrides it.
        let config_path = make_temp_config("[subscription]\nno_sync_before = \"2024-06-15\"\n");
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(
            cfg.subscription_no_sync_before,
            chrono::NaiveDate::from_ymd_opt(2024, 6, 15).unwrap()
        );
        fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_db_skip_default_playlist_default_and_toml_override() {
        let key = test_key();
        set_test_env_key(&key);

        // Defaults to false.
        let cli = Cli::parse_from(["halogen-server"]);
        assert!(!Config::resolve(&cli).unwrap().db_skip_default_playlist);

        // TOML can enable it.
        let config_path = make_temp_config("[db]\nskip_default_playlist = true\n");
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        assert!(Config::resolve(&cli).unwrap().db_skip_default_playlist);
        fs::remove_file(&config_path).ok();
    }

    // The auto-playlist insert-position default resolves from every source:
    // defaults (false) < TOML < env < CLI < the runtime overrides file (it's on
    // the allowlist — unlike the watchdog knobs below).
    #[test]
    fn test_auto_playlist_add_to_start_all_sources() {
        let key = test_key();
        set_test_env_key(&key);

        // Defaults to false (append at the end).
        let cli = Cli::parse_from(["halogen-server"]);
        assert!(
            !Config::resolve(&cli)
                .unwrap()
                .subscription_auto_playlist_add_to_start
        );

        // TOML can enable it.
        let config_path = make_temp_config("[subscription]\nauto_playlist_add_to_start = true\n");
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        assert!(
            Config::resolve(&cli)
                .unwrap()
                .subscription_auto_playlist_add_to_start
        );
        fs::remove_file(&config_path).ok();

        // Env can enable it.
        unsafe {
            env::set_var(
                format!("HALOGEN_SUBSCRIPTION_AUTO_PLAYLIST_ADD_TO_START_{key}"),
                "true",
            )
        };
        let cli = Cli::parse_from(["halogen-server"]);
        assert!(
            Config::resolve(&cli)
                .unwrap()
                .subscription_auto_playlist_add_to_start
        );
        unsafe {
            env::remove_var(format!(
                "HALOGEN_SUBSCRIPTION_AUTO_PLAYLIST_ADD_TO_START_{key}"
            ))
        };

        // CLI flag enables it.
        let cli = Cli::parse_from([
            "halogen-server",
            "--subscription-auto-playlist-add-to-start",
        ]);
        assert!(
            Config::resolve(&cli)
                .unwrap()
                .subscription_auto_playlist_add_to_start
        );

        // The runtime overrides file applies it (allowlisted) and records it.
        let ov = make_temp_overrides("[subscription]\nauto_playlist_add_to_start = true\n");
        let cli = Cli::parse_from([
            "halogen-server",
            "--config-overrides-path",
            ov.to_str().unwrap(),
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert!(cfg.subscription_auto_playlist_add_to_start);
        assert!(
            cfg.overridden_fields
                .iter()
                .any(|f| f == "subscription_auto_playlist_add_to_start")
        );
        fs::remove_file(&ov).ok();
    }

    #[test]
    fn test_disable_polling_service_toml() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config("[server]\ndisable_polling_service = true\n");
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert!(cfg.server_disable_polling_service);
        fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_disable_polling_service_env() {
        let key = test_key();
        set_test_env_key(&key);
        unsafe {
            env::set_var(
                format!("HALOGEN_SERVER_DISABLE_POLLING_SERVICE_{key}"),
                "true",
            )
        };
        let cli = Cli::parse_from(["halogen-server"]);
        let cfg = Config::resolve(&cli).unwrap();
        assert!(cfg.server_disable_polling_service);
    }

    #[test]
    fn test_disable_polling_service_cli() {
        let key = test_key();
        set_test_env_key(&key);
        let cli = Cli::parse_from(["halogen-server", "--server-disable-polling-service"]);
        let cfg = Config::resolve(&cli).unwrap();
        assert!(cfg.server_disable_polling_service);
    }

    // Download-watchdog knobs (`subscription_download_stuck_after` /
    // `subscription_download_max_attempts`) resolve from each boot source: TOML,
    // env, and CLI. `stuck_after` is seconds → `Duration`. These are boot config
    // only — intentionally NOT in the runtime-overrides allowlist (`apply_overrides`).
    #[test]
    fn test_download_watchdog_knobs_toml() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config(
            "[subscription]\ndownload_stuck_after = 120\ndownload_max_attempts = 7\n",
        );
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(
            cfg.subscription_download_stuck_after,
            Duration::from_secs(120)
        );
        assert_eq!(cfg.subscription_download_max_attempts, 7);
        fs::remove_file(&config_path).ok();
    }

    #[test]
    fn test_download_watchdog_knobs_env() {
        let key = test_key();
        set_test_env_key(&key);
        unsafe {
            env::set_var(
                format!("HALOGEN_SUBSCRIPTION_DOWNLOAD_STUCK_AFTER_{key}"),
                "300",
            );
            env::set_var(
                format!("HALOGEN_SUBSCRIPTION_DOWNLOAD_MAX_ATTEMPTS_{key}"),
                "4",
            );
        };
        let cli = Cli::parse_from(["halogen-server"]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(
            cfg.subscription_download_stuck_after,
            Duration::from_secs(300)
        );
        assert_eq!(cfg.subscription_download_max_attempts, 4);
    }

    #[test]
    fn test_download_watchdog_knobs_cli() {
        let key = test_key();
        set_test_env_key(&key);
        let cli = Cli::parse_from([
            "halogen-server",
            "--subscription-download-stuck-after",
            "90",
            "--subscription-download-max-attempts",
            "2",
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(
            cfg.subscription_download_stuck_after,
            Duration::from_secs(90)
        );
        assert_eq!(cfg.subscription_download_max_attempts, 2);
    }

    // Gap 1: an empty `auth_token_secret` after all layering makes `resolve`
    // return Err. Setting the keyed env var to an empty string defeats the
    // test-only fallback in `config_env_var` (the keyed lookup succeeds with
    // ""), while `apply_env`'s non-empty guard then skips the assignment — so
    // the secret stays empty and validation fails.
    #[test]
    fn test_missing_auth_token_secret_rejected() {
        let key = test_key();
        set_test_env_key(&key);
        unsafe { env::set_var(format!("HALOGEN_AUTH_TOKEN_SECRET_{key}"), "") };
        let cli = Cli::parse_from(["halogen-server"]);
        let result = Config::resolve(&cli);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("auth token secret is required")
        );
    }

    // Gap 2: a config file with broken TOML syntax surfaces as an Err from
    // `resolve` (parse failure in `ConfigFile::from_path`), not a panic.
    #[test]
    fn test_invalid_toml_syntax_rejected() {
        let key = test_key();
        set_test_env_key(&key);
        // Unterminated string / dangling key — invalid TOML.
        let config_path = make_temp_config("[server]\nlisten_address = \n");
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let result = Config::resolve(&cli);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to parse config file"));
        fs::remove_file(&config_path).ok();
    }

    // Gap 3a: public file server config resolves from the TOML `[public]`
    // section into the resolved `Config`.
    #[test]
    fn test_public_server_config_from_toml() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config(
            "[public]\nenable = true\nroot = \"/tmp/public-toml\"\nurl_path = \"/app\"\n",
        );
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert!(cfg.enable_public_server);
        assert_eq!(cfg.public_root, Some(PathBuf::from("/tmp/public-toml")));
        assert_eq!(cfg.public_url_path, "/app");
        fs::remove_file(&config_path).ok();
    }

    // Gap 3b: public file server config precedence — env overrides the TOML
    // root, and CLI overrides the url_path. Confirms each public field layers
    // like the rest of the config.
    #[test]
    fn test_public_server_config_precedence() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config(
            "[public]\nenable = true\nroot = \"/tmp/public-toml\"\nurl_path = \"/toml\"\n",
        );
        unsafe { env::set_var(format!("HALOGEN_PUBLIC_ROOT_{key}"), "/tmp/public-env") };
        let cli = Cli::parse_from([
            "halogen-server",
            "-c",
            config_path.to_str().unwrap(),
            "--public-url-path",
            "/cli",
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert!(cfg.enable_public_server);
        // env wins over the TOML root.
        assert_eq!(cfg.public_root, Some(PathBuf::from("/tmp/public-env")));
        // CLI wins over the TOML url_path.
        assert_eq!(cfg.public_url_path, "/cli");
        fs::remove_file(&config_path).ok();
    }

    // Gap 4: a non-numeric value in the port env var does NOT error. `apply_env`
    // parses with `if let Ok(..)`, so a bad value is silently ignored and the
    // port keeps its prior value (here the default 8080).
    #[test]
    fn test_non_numeric_port_env_ignored() {
        let key = test_key();
        set_test_env_key(&key);
        unsafe { env::set_var(format!("HALOGEN_LISTEN_PORT_{key}"), "not-a-number") };
        let cli = Cli::parse_from(["halogen-server"]);
        let cfg = Config::resolve(&cli).unwrap();
        // Unparseable port is ignored; default is retained.
        assert_eq!(cfg.listen_port, 8080);
    }

    // Gap 5a: admin username/password resolve from the TOML `[admin]` section.
    #[test]
    fn test_admin_config_from_toml() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path =
            make_temp_config("[admin]\nusername = \"root\"\npassword = \"hunter2\"\n");
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.admin_username, Some("root".to_string()));
        assert_eq!(cfg.admin_password, Some("hunter2".to_string()));
        fs::remove_file(&config_path).ok();
    }

    // Gap 5b: admin credentials from env override the TOML values.
    #[test]
    fn test_admin_config_env_overrides_toml() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path =
            make_temp_config("[admin]\nusername = \"toml-user\"\npassword = \"toml-pass\"\n");
        unsafe { env::set_var(format!("HALOGEN_ADMIN_USERNAME_{key}"), "env-user") };
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.admin_username, Some("env-user".to_string()));
        // password not overridden by env, so the TOML value remains.
        assert_eq!(cfg.admin_password, Some("toml-pass".to_string()));
        fs::remove_file(&config_path).ok();
    }

    // Gap 5c: the OPML file path resolves from the TOML `[opml]` section.
    #[test]
    fn test_opml_file_from_toml() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config("[opml]\nfile = \"/tmp/subs.opml\"\n");
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.opml_file, Some(PathBuf::from("/tmp/subs.opml")));
        fs::remove_file(&config_path).ok();
    }

    // ── Config overrides ──────────────────────────────────────────────────

    fn make_temp_overrides(content: &str) -> PathBuf {
        static OV_COUNTER: AtomicUsize = AtomicUsize::new(0);
        let counter = OV_COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = env::temp_dir().join(format!(
            "halogen_test_overrides_{}_{counter}.toml",
            process::id()
        ));
        let mut f = fs::File::create(&path).unwrap();
        io::Write::write_all(&mut f, content.as_bytes()).unwrap();
        drop(f);
        path
    }

    // The overrides file is layered LAST, so it beats even a CLI flag, and the
    // changed field is recorded for reporting.
    #[test]
    fn test_overrides_beat_cli() {
        let key = test_key();
        set_test_env_key(&key);
        let ov = make_temp_overrides("[auth]\ntoken_expiry_minutes = 999\n");
        let cli = Cli::parse_from([
            "halogen-server",
            "--auth-token-expiry-minutes",
            "111",
            "--config-overrides-path",
            ov.to_str().unwrap(),
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.auth_token_expiry_minutes, 999);
        assert!(
            cfg.overridden_fields
                .iter()
                .any(|f| f == "auth_token_expiry_minutes")
        );
        assert_eq!(cfg.config_overrides_loaded_from, Some(ov.clone()));
        fs::remove_file(&ov).ok();
    }

    // Non-allowlisted keys in the overrides file are ignored (here the JWT
    // secret) but recorded as rejected; allowlisted keys alongside still apply.
    #[test]
    fn test_overrides_allowlist_rejects_secret() {
        let key = test_key();
        set_test_env_key(&key);
        let ov =
            make_temp_overrides("[auth]\ntoken_secret = \"sneaky\"\ntoken_expiry_minutes = 42\n");
        let cli = Cli::parse_from([
            "halogen-server",
            "--config-overrides-path",
            ov.to_str().unwrap(),
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert_eq!(cfg.auth_token_expiry_minutes, 42);
        // Secret is NOT overridable: it keeps the test fallback value.
        assert_eq!(cfg.auth_token_secret, "test-secret");
        assert!(
            cfg.rejected_override_keys
                .iter()
                .any(|k| k == "auth.token_secret")
        );
        fs::remove_file(&ov).ok();
    }

    // The disable flag skips loading entirely — the override value is not applied
    // and nothing is recorded as loaded.
    #[test]
    fn test_overrides_disable_skips_loading() {
        let key = test_key();
        set_test_env_key(&key);
        let ov = make_temp_overrides("[auth]\ntoken_expiry_minutes = 777\n");
        let cli = Cli::parse_from([
            "halogen-server",
            "--config-overrides-disable",
            "--config-overrides-path",
            ov.to_str().unwrap(),
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert!(cfg.config_overrides_disable);
        assert_eq!(cfg.auth_token_expiry_minutes, 60 * 24 * 7);
        assert!(cfg.config_overrides_loaded_from.is_none());
        assert!(cfg.overridden_fields.is_empty());
        fs::remove_file(&ov).ok();
    }

    // With no explicit override path, it defaults to `config.overrides.toml`
    // beside the `--config` file.
    #[test]
    fn test_overrides_path_defaults_beside_config() {
        let key = test_key();
        set_test_env_key(&key);
        let config_path = make_temp_config("[server]\nlisten_port = 8080\n");
        let cli = Cli::parse_from(["halogen-server", "-c", config_path.to_str().unwrap()]);
        let cfg = Config::resolve(&cli).unwrap();
        let expected = config_path.parent().unwrap().join("config.overrides.toml");
        assert_eq!(cfg.config_overrides_path, Some(expected));
        fs::remove_file(&config_path).ok();
    }

    // A malformed overrides file is non-fatal: `resolve` still succeeds, the
    // error is recorded for a post-init warning, and nothing is marked loaded.
    #[test]
    fn test_malformed_overrides_tolerated() {
        let key = test_key();
        set_test_env_key(&key);
        let ov = make_temp_overrides("[auth]\ntoken_expiry_minutes = \n");
        let cli = Cli::parse_from([
            "halogen-server",
            "--config-overrides-path",
            ov.to_str().unwrap(),
        ]);
        let cfg = Config::resolve(&cli).unwrap();
        assert!(cfg.config_overrides_load_error.is_some());
        assert!(cfg.config_overrides_loaded_from.is_none());
        fs::remove_file(&ov).ok();
    }

    // read → write → read round-trip: a missing file reads as the empty set, and
    // a written override set (the endpoint replaces wholesale) is re-readable.
    #[test]
    fn test_overrides_read_write_roundtrip() {
        use halogen_wire::ConfigOverridesData;
        let path = env::temp_dir().join(format!(
            "halogen_ovrt_{}_{}.toml",
            process::id(),
            test_key()
        ));
        fs::remove_file(&path).ok();

        // Missing file → empty set.
        let base = read_overrides(&path).unwrap();
        assert!(base.auth_token_expiry_minutes.is_none());

        let overrides = ConfigOverridesData {
            auth_token_expiry_minutes: Some(15),
            episode_playback_complete_percentage: Some(10),
            ..Default::default()
        };
        write_overrides(&path, &overrides).unwrap();

        let back = read_overrides(&path).unwrap();
        assert_eq!(back.auth_token_expiry_minutes, Some(15));
        assert_eq!(back.episode_playback_complete_percentage, Some(10));
        fs::remove_file(&path).ok();
    }
}
