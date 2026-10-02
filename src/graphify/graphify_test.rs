use super::prepare_worktree;
use crate::orchestrator::app::{set_switch, GRAPHIFY};
use crate::orchestrator::write_file;
use crate::tempdir::TempDir;
use crate::tools::fake::Fake;

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
