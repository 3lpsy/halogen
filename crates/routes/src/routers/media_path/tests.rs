use super::confined;
use halogen_fixture::test_support::TestRoot;
use std::fs;

#[test]
fn accepts_files_inside_the_root() {
    let mut root = TestRoot::new("media_path_inside");
    let media = root.path().join("media");
    fs::create_dir_all(media.join("art")).unwrap();

    let audio = media.join("42.mp3");
    fs::write(&audio, b"x").unwrap();
    assert!(
        confined(&media, &audio).is_some(),
        "audio under root serves"
    );

    let art = media.join("art").join("p.png");
    fs::write(&art, b"x").unwrap();
    assert!(
        confined(&media, &art).is_some(),
        "art subdir is inside root"
    );

    root.mark_success();
}

#[test]
fn rebases_a_stale_absolute_path_onto_the_root() {
    let mut root = TestRoot::new("media_path_rebase");
    let media = root.path().join("media");
    fs::create_dir_all(&media).unwrap();
    fs::write(media.join("7.mp3"), b"x").unwrap();

    // The path a previous app-container location baked into the DB.
    let stale = root
        .path()
        .join("old-container")
        .join("media")
        .join("7.mp3");
    assert!(
        super::confined(&media, &stale).is_none(),
        "stale absolute path must not confine"
    );
    let rebased = super::confined_or_rebased(&media, &stale).expect("rebase serves");
    assert!(rebased.ends_with("7.mp3"));

    root.mark_success();
}

#[test]
fn rejects_a_file_outside_the_root() {
    let mut root = TestRoot::new("media_path_outside");
    let media = root.path().join("media");
    fs::create_dir_all(&media).unwrap();
    // A real, readable file that lives outside media_root.
    let outside = root.path().join("secret.txt");
    fs::write(&outside, b"secret").unwrap();
    assert!(
        confined(&media, &outside).is_none(),
        "an existing file outside media_root must be refused"
    );
    root.mark_success();
}

#[test]
fn rejects_a_traversal_escape() {
    let mut root = TestRoot::new("media_path_traversal");
    let media = root.path().join("media");
    fs::create_dir_all(&media).unwrap();
    let outside = root.path().join("secret.txt");
    fs::write(&outside, b"secret").unwrap();
    // `<media_root>/../secret.txt` resolves outside the root.
    let sneaky = media.join("..").join("secret.txt");
    assert!(
        confined(&media, &sneaky).is_none(),
        "a `../` escape must be refused"
    );
    root.mark_success();
}
