//! 緊急退避の公開物の `sensitive` (docs/02 §11.2.1 の「機密」、docs/07 §5.12.2)。

use eve_testkit::Harness;

fn snapshot_sensitive(path: Option<&str>) -> bool {
    let mut h = match path {
        Some(p) => {
            let mut h = Harness::new().with_file(p, b"K=1\n");
            h.open(p);
            h.run_jobs();
            h
        }
        None => Harness::new(),
    };
    h.text("x");
    let doc = h.doc_id();
    h.editor
        .emergency_snapshot(doc)
        .expect("dirty な文書は公開物がある")
        .sensitive
}

#[test]
fn 秘密ファイルのパターンに一致する文書は_sensitive_を立てる() {
    for p in [
        "/w/.env",
        "/w/.env.local",
        "/w/server.pem",
        "/w/id_rsa",
        "/w/.ssh/config",
    ] {
        assert!(snapshot_sensitive(Some(p)), "{p}");
    }
}

#[test]
fn 一致しない文書と無題の文書は立てない() {
    for p in ["/w/readme.md", "/w/notes.txt", "/w/dotenv.txt"] {
        assert!(!snapshot_sensitive(Some(p)), "{p}");
    }
    assert!(!snapshot_sensitive(None));
}
