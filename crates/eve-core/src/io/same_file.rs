//! 同じファイルの判定 (SAF-28、docs/03 §12.7.6): 二重オープン (03 §9.2 手順 2b) と名前を付けて保存の保存先 (03 §9.7 (a) の
//! 1 / 1b) が使う比べ方と、ジョブの側で canonical のパスを求める関数。
//!
//! 不変条件: 開いている文書の集合の中で、同じファイルを指す文書は 1 つだけ (03 §9.2 手順 2b)。2 つの文書が 1 つのファイルを
//! 指すと、片方の保存がもう片方の外部変更になり、照合 (03 §9.5) と保存点がずれて、古い本文で上書きし得る。比べ方は
//! 「別のファイルを同じと判定しない」側に倒す: 識別 (`FileId`) は FS が値を使い回さないと分かっているときだけ使い、
//! そうでなければ canonical のパスの文字列が等しいときだけ同じとする (大文字小文字を自前で畳まない、05 §3.1)。
//!
//! spec: 03 §12.7.6, 03 §9.2, 05 §5.4

use super::fs::{FileId, FileSystem};
use super::fspath::FsPath;

/// 同じファイルの判定に使う、ファイルの識別 (03 §12.7.6 の 0)。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FileIdentity {
    /// ファイルの識別。取れない FS・まだ無いファイルは `None`。
    pub file_id: Option<FileId>,
    /// `file_id` を判定に使ってよい (`FileMeta.file_id_unique`)。
    pub file_id_unique: bool,
    /// canonical のパス。取れなければ正規化したパス (05 §3.2)。無題の文書・`Identity` が届く前の文書は `None`。
    pub canonical: Option<FsPath>,
}

impl FileIdentity {
    /// `self` と `other` が同じファイルか (03 §12.7.6 の 0): どちらも信頼できる `FileId` を持ち、それが等しい、**または**
    /// どちらも canonical のパスを持ち、Windows の verbatim の接頭辞を外した形が等しい。
    pub(crate) fn same_file(&self, other: &FileIdentity) -> bool {
        let by_id = match (self.file_id, other.file_id) {
            (Some(a), Some(b)) => self.file_id_unique && other.file_id_unique && a == b,
            _ => false,
        };
        by_id
            || match (&self.canonical, &other.canonical) {
                (Some(a), Some(b)) => canonical_key(a) == canonical_key(b),
                _ => false,
            }
    }
}

/// 比べる形の canonical のパス: Windows の verbatim の接頭辞 (`\\?\`・`\\?\UNC\`) を外す (05 §3.3 の表示用の形は 260
/// 文字を超えるパスで接頭辞を残すので、そのままでは「ルート + 名前」で作ったパスと揃わない)。UTF-8 で表せないパスは
/// そのまま比べる (接頭辞が残れば一致しないだけで、別のファイルを同じとは判定しない)。
pub(crate) fn canonical_key(p: &FsPath) -> std::path::PathBuf {
    let raw = p.as_path();
    let Some(s) = raw.to_str() else {
        return raw.to_path_buf();
    };
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        std::path::PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        std::path::PathBuf::from(rest)
    } else {
        raw.to_path_buf()
    }
}

/// ジョブの側で求める canonical のパス (03 §12.7.6 の 0 / 2): `path` を canonicalize したもの。`path` が無ければ (新規の
/// ファイル・名前を付けて保存の新しい保存先・ダングリングのリンク)、親ディレクトリを canonicalize してファイル名を付けた
/// もの。どちらも取れなければ `None` (主スレッドが正規化したパスで代える)。
pub(crate) fn canonical_of(fs: &dyn FileSystem, path: &FsPath) -> Option<FsPath> {
    if let Ok(c) = fs.canonicalize(path) {
        return Some(c);
    }
    let name = path.as_path().file_name()?;
    let parent = path.parent()?;
    fs.canonicalize(&parent).ok().map(|dir| dir.join(name))
}

impl crate::editor::state::EditorState {
    /// 開くジョブの識別 (`JobResult::Identity`、03 §9.2 手順 2b・§12.7.6 の 1) を受けた。新しく開く読込 (`OpenKind::Open`)
    /// で、ほかの文書 (読込中・確認待ちを含む) と同じファイルなら、新しい文書を閉じて読込を取り消し、既存の文書を開いた
    /// 時点のペインに出す。違えば (再読込・開き直しでも) 文書に識別を覚える。
    pub(crate) fn on_open_identity(&mut self, job: eve_view::JobId, mut ident: FileIdentity) {
        let Some(super::pending::Pending::Open {
            doc,
            path,
            replace,
            kind,
            ..
        }) = self.io.pending.get(&job)
        else {
            return;
        };
        let (doc, replace, kind) = (*doc, *replace, *kind);
        // canonicalize できなかったときは正規化したパスで代える (03 §12.7.6 の 0)
        if ident.canonical.is_none() {
            ident.canonical = Some(path.clone());
        }
        if kind == super::open::OpenKind::Open
            && let Some(existing) = self.document_same_as(&ident, Some(doc))
        {
            self.open_already_open(job, doc, existing, replace);
            return;
        }
        if let Some(d) = self.docs.documents.get_mut(&doc) {
            d.canonical = ident.canonical;
            d.file_id_unique = ident.file_id_unique;
            d.loading_file_id = ident.file_id;
        }
    }

    /// 新しく開いた文書 `doc` が既存の文書 `existing` と同じファイルだった: 読込を取り消して `doc` を閉じ、`existing` を
    /// 開いた時点のペイン (切り替え元が無ければフォーカスのペイン) に出す。`existing` がそのペインに出ていた文書なら、
    /// 何も変えずに「既に開いています」と示す (03 §9.2 の「置き換えの手順」の末尾)。読込中の `doc` には未保存の変更が
    /// 無いので、確認なしで閉じてよい (03 §9.2 の閉じる手順 (c))。
    fn open_already_open(
        &mut self,
        job: eve_view::JobId,
        doc: eve_view::DocumentId,
        existing: eve_view::DocumentId,
        replace: Option<super::open::OpenFrom>,
    ) {
        self.io.pending.remove(&job);
        self.ui.continuations.remove(&job);
        self.jobs.cancel(job);
        // 起動時の `path:行:桁` は、まだ読んでいる既存の文書に引き継ぐ (読み終えた文書の位置は動かさない)
        if let Some(jump) = self.io.open_jumps.remove(&doc)
            && self
                .document(existing)
                .is_some_and(|d| d.load_state != crate::document::LoadState::Ready)
        {
            self.io.open_jumps.entry(existing).or_insert(jump);
        }
        let pane = replace.map(|r| r.pane).or_else(|| self.focused_pane());
        if replace.is_some_and(|r| r.doc == existing) {
            let shown = self
                .document(existing)
                .and_then(|d| d.path.as_ref())
                .map(FsPath::display)
                .unwrap_or_default();
            self.message(
                eve_view::MessageLevel::Info,
                crate::ui_model::keys::INFO_OPEN_ALREADY_OPEN,
                &shown,
            );
        }
        if self.panes_showing(doc).is_empty() {
            // 表示していない (読込の完了で切り替える前): 既存の文書をそのペインに出してから閉じる。オーバーレイ・検索バーの
            // 間は表示を動かさない (03 §9.2.1 の「表示の切り替え」と同じ)
            if let Some(at) = pane
                && !self.auto_switch_blocked()
            {
                self.show_document_in(at, existing);
            }
            self.close_document(doc);
        } else {
            // 表示している (起動時の最初の文書など): 表示しているペインを既存の文書へ替えてから閉じる
            self.close_shown_document(doc, pane.map(|p| (p, existing)));
        }
        self.mark_dirty_render();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(index: u128) -> Option<FileId> {
        Some(FileId { volume: 1, index })
    }

    fn ident(file_id: Option<FileId>, unique: bool, canonical: Option<&str>) -> FileIdentity {
        FileIdentity {
            file_id,
            file_id_unique: unique,
            canonical: canonical.map(FsPath::from),
        }
    }

    #[test]
    fn 信頼できる識別が等しければ同じ() {
        let a = ident(id(5), true, Some("/w/a.txt"));
        let b = ident(id(5), true, Some("/w/hardlink.txt"));
        assert!(a.same_file(&b), "ハードリンク: パスは違っても識別が等しい");
    }

    #[test]
    fn 信頼できない識別は使わない() {
        // exFAT・ネットワーク: 識別が重なっても、パスが違えば別のファイル
        let a = ident(id(5), false, Some("/w/a.txt"));
        let b = ident(id(5), true, Some("/w/b.txt"));
        assert!(!a.same_file(&b));
        assert!(!b.same_file(&a), "どちらか一方でも信頼できなければ使わない");
    }

    #[test]
    fn 識別が無くても_canonical_が等しければ同じ() {
        let a = ident(None, false, Some("/w/a.txt"));
        let b = ident(id(9), true, Some("/w/a.txt"));
        assert!(a.same_file(&b));
    }

    #[test]
    fn 識別が違っても_canonical_が等しければ同じ() {
        // 原子的保存で識別が変わった直後 (05 §5.4) も、同じパスなら同じファイル
        let a = ident(id(1), true, Some("/w/a.txt"));
        let b = ident(id(2), true, Some("/w/a.txt"));
        assert!(a.same_file(&b));
    }

    #[test]
    fn どちらも持たなければ同じとしない() {
        assert!(!FileIdentity::default().same_file(&FileIdentity::default()));
        let a = ident(None, false, None);
        let b = ident(None, false, Some("/w/a.txt"));
        assert!(!a.same_file(&b));
    }

    #[test]
    fn 大文字小文字は畳まない() {
        // canonical のパスは OS が実際の綴りに直す (Windows)。自前で畳むと、区別する FS で別のファイルを同じとする
        let a = ident(None, false, Some("/w/A.txt"));
        let b = ident(None, false, Some("/w/a.txt"));
        assert!(!a.same_file(&b));
    }

    #[test]
    fn verbatim_の接頭辞を外して比べる() {
        let a = ident(None, false, Some(r"\\?\C:\w\a.txt"));
        let b = ident(None, false, Some(r"C:\w\a.txt"));
        assert!(a.same_file(&b));
        let a = ident(None, false, Some(r"\\?\UNC\srv\sh\a.txt"));
        let b = ident(None, false, Some(r"\\srv\sh\a.txt"));
        assert!(a.same_file(&b));
    }
}
