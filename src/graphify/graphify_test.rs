use super::{handled, is_new, latest, pass, prepare_worktree, set_handled};
use crate::orchestrator::app::{set_switch, GRAPHIFY};
use crate::orchestrator::state::State;
use crate::orchestrator::write_file;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;
use std::sync::atomic::{AtomicBool, Ordering};

/// With no Orchestrator: off runs nothing; on, the checkout's graph is
/// copied in when it has one, and graphify update . runs in the worktree, bounded.
#[test]
fn prepare_worktree_copies_the_checkouts_graph_and_updates_it_in_the_worktree() {
    let checkout = TempDir::new();
    let tree = TempDir::new();
    let worktree = tree.path().to_path_buf();
    let calls = |checkout: &TempDir| {
        let worktree = worktree.clone();
        let fake = Fake::new(move |dir, _| {
            assert_eq!(dir, worktree);
            Ok(String::new())
        });
        prepare_worktree(&*fake, checkout.path(), tree.path()).unwrap();
        fake.calls()
    };

    assert!(calls(&checkout).is_empty(), "off ran something");

    set_switch(checkout.path(), &GRAPHIFY, true).unwrap();
    assert_eq!(calls(&checkout), ["graphify update . [within 300s]"]);

    write_file(&checkout.path().join("graphify-out/graph.json"), "{}");
    assert_eq!(
        calls(&checkout),
        [
            format!(
                "cp -R {}/graphify-out {}/graphify-out",
                checkout.path().display(),
                tree.path().display()
            ),
            "graphify update . [within 300s]".to_string(),
        ]
    );
}

#[test]
fn the_highest_merged_tag_is_the_highest_vxyz_semver_reads() {
    let tags = "v1.2.0\nv1.3.1\nv1.10.0\nv2.0.0-rc1\nnotes\n";
    assert_eq!(latest(tags).as_deref(), Some("v1.10.0"));
    assert_eq!(latest(""), None);
    assert_eq!(latest("notes\nv2.0.0-rc1\n"), None);
}

#[test]
fn a_tag_is_new_when_its_major_or_minor_differs_from_the_one_handled() {
    assert!(!is_new(Some((1, 3)), Some("v1.3.5")), "a patch-only tag");
    assert!(is_new(Some((1, 3)), Some("v1.4.0")));
    assert!(is_new(Some((1, 3)), Some("v2.3.0")));
    assert!(is_new(None, Some("v1.3.0")), "nothing handled yet");
    assert!(!is_new(None, None), "a repo with no tags");
    assert!(!is_new(Some((1, 3)), None));
}

#[test]
fn the_handled_version_is_kept_in_orqadence_local_and_outlives_a_saved_empty_state() {
    let repo = TempDir::new();
    assert_eq!(handled(repo.path()), None);
    set_handled(repo.path(), "v1.3.2").unwrap();
    assert_eq!(
        std::fs::read_to_string(repo.path().join(".orqadence-local/graphify-docs-pass")).unwrap(),
        "1.3"
    );
    assert_eq!(handled(repo.path()), Some((1, 3)));
    State::default().save(repo.path()).unwrap();
    assert_eq!(handled(repo.path()), Some((1, 3)));
    assert!(set_handled(repo.path(), "v2.0.0-rc1").is_err());
    assert_eq!(handled(repo.path()), Some((1, 3)));
}

#[test]
fn an_unset_origin_head_is_set_once_and_the_tags_read_again() {
    let repo = TempDir::new();
    let set = AtomicBool::new(false);
    let fake = Fake::new(move |_, argv| match argv.join(" ").as_str() {
        "git remote set-head origin --auto" => {
            set.store(true, Ordering::SeqCst);
            Ok(String::new())
        }
        "git tag --merged origin/HEAD --list v*" if !set.load(Ordering::SeqCst) => {
            Err("malformed object name origin/HEAD".to_string())
        }
        "git tag --merged origin/HEAD --list v*" => Ok("v1.3.0\n".to_string()),
        _ => Ok(String::new()),
    });
    let (failed, tag) = pass(&*fake, repo.path());
    assert_eq!(failed, Vec::<String>::new());
    assert_eq!(tag.as_deref(), Some("v1.3.0"));
    let calls = fake.calls();
    let tail = &calls[calls.len() - 3..];
    assert_eq!(
        tail,
        [
            "git tag --merged origin/HEAD --list v*",
            "git remote set-head origin --auto",
            "git tag --merged origin/HEAD --list v*",
        ]
    );

    // With 1.3 handled, the same tag is not new.
    set_handled(repo.path(), "v1.3.0").unwrap();
    assert_eq!(pass(&*fake, repo.path()).1, None);
}

#[test]
fn graphify_from_pipx_upgrades_with_pipx_and_from_uv_with_uv() {
    let repo = TempDir::new();
    let upgrade = |list: &'static str| {
        let fake = Fake::new(move |_, argv| match argv.join(" ").as_str() {
            "which uv" if list.is_empty() => Err("no uv".to_string()),
            "uv tool list" => Ok(list.to_string()),
            _ => Ok(String::new()),
        });
        pass(&*fake, repo.path());
        fake.calls()
            .into_iter()
            .filter(|c| c.ends_with("upgrade graphifyy"))
            .collect::<Vec<_>>()
    };
    assert_eq!(upgrade("ruff v0.6.0\n- ruff\n"), ["pipx upgrade graphifyy"]);
    assert_eq!(upgrade(""), ["pipx upgrade graphifyy"], "no uv");
    assert_eq!(
        upgrade("graphifyy v0.4.1\n- graphify\n"),
        ["uv tool upgrade graphifyy"]
    );
}

#[test]
fn a_failed_uv_listing_is_one_line_and_upgrades_and_installs_nothing() {
    let repo = TempDir::new();
    let fake = Fake::new(|_, argv| match argv.join(" ").as_str() {
        "uv tool list" => Err("broken".to_string()),
        _ => Ok(String::new()),
    });
    let (failed, _) = pass(&*fake, repo.path());
    assert_eq!(failed.len(), 1, "{failed:?}");
    assert!(failed[0].contains("uv tool list"), "{failed:?}");
    let calls = fake.calls();
    assert!(
        !calls
            .iter()
            .any(|c| c.ends_with("upgrade graphifyy") || c.starts_with("graphify install")),
        "{calls:?}"
    );
}
