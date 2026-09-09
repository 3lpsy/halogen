use sea_orm_migration::prelude::*;
#[derive(DeriveMigrationName)]
pub struct Migration;
#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("CREATE TABLE sync_change (sequence INTEGER PRIMARY KEY AUTOINCREMENT, resource TEXT NOT NULL CHECK(resource IN ('podcasts','episodes','playbacks','playlists','podcast_auto_playlists')), resource_id INTEGER NOT NULL CHECK(resource_id > 0), actor_id INTEGER NOT NULL CHECK(actor_id > 0), deleted INTEGER NOT NULL CHECK(deleted IN (0,1)), created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP); CREATE INDEX sync_change_actor_sequence ON sync_change(actor_id, sequence); CREATE INDEX sync_change_created_at ON sync_change(created_at)").await?;
        db.execute_unprepared("CREATE TRIGGER sync_playlist_insert AFTER INSERT ON playlist BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playlists', NEW.id, NEW.user_id, 0 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_playlist_update AFTER UPDATE ON playlist BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playlists', NEW.id, NEW.user_id, 0 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_playlist_delete AFTER DELETE ON playlist BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playlists', OLD.id, OLD.user_id, 1 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_playback_insert AFTER INSERT ON playback BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playbacks', NEW.episode_id, NEW.user_id, 0 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_playback_update AFTER UPDATE ON playback BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playbacks', NEW.episode_id, NEW.user_id, 0 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_playback_delete AFTER DELETE ON playback BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playbacks', OLD.episode_id, OLD.user_id, 1 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_user_episode_status_insert AFTER INSERT ON user_episode_status BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', NEW.episode_id, NEW.user_id, 0 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_user_episode_status_update AFTER UPDATE ON user_episode_status BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', NEW.episode_id, NEW.user_id, 0 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_user_episode_status_delete AFTER DELETE ON user_episode_status BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', OLD.episode_id, OLD.user_id, 0 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_podcast_insert AFTER INSERT ON podcast BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcasts', NEW.id, user_id, 0 FROM user_podcast WHERE podcast_id = NEW.id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_podcast_update AFTER UPDATE ON podcast BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcasts', NEW.id, user_id, 0 FROM user_podcast WHERE podcast_id = NEW.id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_podcast_delete BEFORE DELETE ON podcast BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcasts', OLD.id, user_id, 1 FROM user_podcast WHERE podcast_id = OLD.id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_episode_insert AFTER INSERT ON episode BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', NEW.id, user_id, 0 FROM user_podcast WHERE podcast_id = NEW.podcast_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_episode_update AFTER UPDATE ON episode BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', NEW.id, user_id, 0 FROM user_podcast WHERE podcast_id = NEW.podcast_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_episode_delete BEFORE DELETE ON episode BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', OLD.id, user_id, 1 FROM user_podcast WHERE podcast_id = OLD.podcast_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_subscription_insert AFTER INSERT ON user_podcast BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcasts', NEW.podcast_id, NEW.user_id, 0 ;INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', id, NEW.user_id, 0 FROM episode WHERE podcast_id = NEW.podcast_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_subscription_delete AFTER DELETE ON user_podcast BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcasts', OLD.podcast_id, OLD.user_id, 1 ; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_membership_insert AFTER INSERT ON episode_playlist BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playlists', NEW.playlist_id, user_id, 0 FROM playlist WHERE id = NEW.playlist_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_auto_playlist_insert AFTER INSERT ON podcast_auto_playlist BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcast_auto_playlists', NEW.podcast_id, user_id, 0 FROM playlist WHERE id = NEW.playlist_id UNION SELECT 'podcast_auto_playlists', NEW.podcast_id, u.id, 0 FROM user u JOIN user_podcast up ON up.user_id = u.id WHERE u.is_admin = 1 AND up.podcast_id = NEW.podcast_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_config_insert AFTER INSERT ON podcast_config BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcasts', p.id, up.user_id, 0 FROM podcast p JOIN user_podcast up ON up.podcast_id = p.id WHERE p.podcast_config_id = NEW.id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_chapter_insert AFTER INSERT ON episode_chapter BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', NEW.episode_id, up.user_id, 0 FROM episode e JOIN user_podcast up ON up.podcast_id = e.podcast_id WHERE e.id = NEW.episode_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_membership_update AFTER UPDATE ON episode_playlist BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playlists', NEW.playlist_id, user_id, 0 FROM playlist WHERE id = NEW.playlist_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_auto_playlist_update AFTER UPDATE ON podcast_auto_playlist BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcast_auto_playlists', NEW.podcast_id, user_id, 0 FROM playlist WHERE id = NEW.playlist_id UNION SELECT 'podcast_auto_playlists', NEW.podcast_id, u.id, 0 FROM user u JOIN user_podcast up ON up.user_id = u.id WHERE u.is_admin = 1 AND up.podcast_id = NEW.podcast_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_config_update AFTER UPDATE ON podcast_config BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcasts', p.id, up.user_id, 0 FROM podcast p JOIN user_podcast up ON up.podcast_id = p.id WHERE p.podcast_config_id = NEW.id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_chapter_update AFTER UPDATE ON episode_chapter BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', NEW.episode_id, up.user_id, 0 FROM episode e JOIN user_podcast up ON up.podcast_id = e.podcast_id WHERE e.id = NEW.episode_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_membership_delete AFTER DELETE ON episode_playlist BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playlists', OLD.playlist_id, user_id, 0 FROM playlist WHERE id = OLD.playlist_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_auto_playlist_delete AFTER DELETE ON podcast_auto_playlist BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcast_auto_playlists', OLD.podcast_id, user_id, 0 FROM playlist WHERE id = OLD.playlist_id UNION SELECT 'podcast_auto_playlists', OLD.podcast_id, u.id, 0 FROM user u JOIN user_podcast up ON up.user_id = u.id WHERE u.is_admin = 1 AND up.podcast_id = OLD.podcast_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_config_delete BEFORE DELETE ON podcast_config BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'podcasts', p.id, up.user_id, 0 FROM podcast p JOIN user_podcast up ON up.podcast_id = p.id WHERE p.podcast_config_id = OLD.id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_chapter_delete AFTER DELETE ON episode_chapter BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'episodes', OLD.episode_id, up.user_id, 0 FROM episode e JOIN user_podcast up ON up.podcast_id = e.podcast_id WHERE e.id = OLD.episode_id; END").await?;
        db.execute_unprepared("CREATE TRIGGER sync_membership_move AFTER UPDATE ON episode_playlist WHEN OLD.playlist_id != NEW.playlist_id BEGIN INSERT INTO sync_change(resource, resource_id, actor_id, deleted) SELECT 'playlists', OLD.playlist_id, user_id, 0 FROM playlist WHERE id = OLD.playlist_id; END").await?;
        Ok(())
    }
    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let db = manager.get_connection();
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_membership_move")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_chapter_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_config_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_auto_playlist_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_membership_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_chapter_update")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_config_update")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_auto_playlist_update")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_membership_update")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_chapter_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_config_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_auto_playlist_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_membership_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_subscription_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_subscription_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_episode_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_episode_update")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_episode_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_podcast_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_podcast_update")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_podcast_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_user_episode_status_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_user_episode_status_update")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_user_episode_status_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_playback_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_playback_update")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_playback_insert")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_playlist_delete")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_playlist_update")
            .await?;
        db.execute_unprepared("DROP TRIGGER IF EXISTS sync_playlist_insert")
            .await?;
        db.execute_unprepared("DROP TABLE sync_change").await?;
        Ok(())
    }
}
