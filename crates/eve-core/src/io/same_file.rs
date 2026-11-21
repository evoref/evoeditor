//! 同じファイルの判定 (SAF-28、docs/03 §12.7.6): 二重オープン (03 §9.2 手順 2b) と名前を付けて保存の保存先 (03 §9.7 (a) の
//! 1 / 1b) が使う比べ方と、ジョブの側で canonical のパスを求める関数。
//!
//! 不変条件: 開いている文書の集合の中で、同じファイルを指す文書は 1 つだけ (03 §9.2 手順 2b)。2 つの文書が 1 つのファイルを
//! 指すと、片方の保存がもう片方の外部変更になり、照合 (03 §9.5) と保存点がずれて、古い本文で上書きし得る。比べ方は
//! 「別のファイルを同じと判定しない」側に倒す: 識別 (`FileId`) は FS が値を使い回さないと分かっていて、canonical のパスの
//! ボリュームの部分も等しいとき (複製したボリュームは識別が重なる) だけ使い、そうでなければ canonical のパスが等しいとき
//! だけ同じとする (大文字小文字を自前で畳まない、05 §3.1)。canonical を取れずに代わりのパスを使うものは識別を使わない。
//! 文書の古い識別は、ジョブがパスを調べ直して確かめる (`stale_known_ids`)。
//!
//! spec: 03 §12.7.6, 03 §9.2, 05 §5.4

use super::fs::{FileId, FileSystem};
use super::fspath::FsPath;
use crate::jobs::KnownFileId;

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
    /// 借りた形 (比べるときに canonical のパスを複製しない)。
    pub(crate) fn as_ref(&self) -> IdentityRef<'_> {
        IdentityRef {
            file_id: self.file_id,
            file_id_unique: self.file_id_unique,
            canonical: self.canonical.as_ref(),
        }
    }

    /// `self` と `other` が同じファイルか (`IdentityRef::same_file`)。
    #[cfg(test)]
    pub(crate) fn same_file(&self, other: &FileIdentity) -> bool {
        self.as_ref().same_file(other.as_ref())
    }
}

/// `FileIdentity` の借りた形 (文書の欄から複製せずに作る。03 §12.7.6 の 0 の比べ方)。
#[derive(Clone, Copy, Debug)]
pub(crate) struct IdentityRef<'a> {
    /// ファイルの識別。
    pub file_id: Option<FileId>,
    /// `file_id` を判定に使ってよい。
    pub file_id_unique: bool,
    /// canonical のパス。
    pub canonical: Option<&'a FsPath>,
}

impl IdentityRef<'_> {
    /// `self` と `other` が同じファイルか (03 §12.7.6 の 0): どちらも信頼できる `FileId` を持ち、それが等しく、canonical の
    /// パスのボリュームの部分も等しい、**または** どちらも canonical のパスを持ち、Windows の verbatim の接頭辞を外した形が
    /// 等しい。
    pub(crate) fn same_file(self, other: IdentityRef<'_>) -> bool {
        let by_id = match (self.file_id, other.file_id) {
            (Some(a), Some(b)) => {
                self.file_id_unique && other.file_id_unique && a == b && self.same_volume(other)
            }
            _ => false,
        };
        by_id || self.same_canonical(other)
    }

    /// どちらも canonical のパスを持ち、それが等しいか (識別を使わない比べ方)。
    pub(crate) fn same_canonical(self, other: IdentityRef<'_>) -> bool {
        match (self.canonical, other.canonical) {
            (Some(a), Some(b)) => canonical_key(a) == canonical_key(b),
            _ => false,
        }
    }

    /// canonical のパスのボリュームの部分が等しいか (分からなければ `false`)。ディスクの複製・バックアップの VHDX・
    /// シャドウコピーは、ボリュームのシリアル番号と `FileId` が元と重なるので、識別だけでは見分けられない (05 §5.4)。
    fn same_volume(self, other: IdentityRef<'_>) -> bool {
        match (
            self.canonical.and_then(volume_of),
            other.canonical.and_then(volume_of),
        ) {
            (Some(a), Some(b)) => a == b,
            _ => false,
        }
    }
}

/// canonical のパスのボリュームの部分 (03 §12.7.6 の 0)。ASCII の大文字にそろえる。Unix の絶対パスは空 (`st_dev` が識別に
/// 入っているので条件を置かない)。Windows はドライブ文字 (`C:`)・UNC のサーバとシェア・`Volume{…}`・`GLOBALROOT` の
/// 装置名。分からない形 (相対パス・置換文字を含むボリューム) は `None`。
pub(crate) fn volume_of(p: &FsPath) -> Option<String> {
    const BS: char = '\\';
    let key = canonical_key(p);
    let s = key.to_string_lossy();
    if s.starts_with('/') {
        return Some(String::new());
    }
    let b = s.as_bytes();
    // verbatim の接頭辞の後の形 (UTF-8 で表せず `canonical_key` が接頭辞を残したパスも、ここで同じ形にそろえる)
    let device = |first: &str, it: &mut std::str::Split<'_, char>| -> Option<String> {
        let fb = first.as_bytes();
        if fb.len() == 2 && fb[0].is_ascii_alphabetic() && fb[1] == b':' {
            Some(first.to_owned())
        } else if first.eq_ignore_ascii_case("UNC") {
            Some(format!("UNC{BS}{}{BS}{}", it.next()?, it.next()?))
        } else if first.eq_ignore_ascii_case("GLOBALROOT") {
            // `GLOBALROOT\Device\HarddiskVolumeShadowCopy1` の 3 つ
            Some(format!("{first}{BS}{}{BS}{}", it.next()?, it.next()?))
        } else if first
            .get(..7)
            .is_some_and(|p| p.eq_ignore_ascii_case("Volume{"))
        {
            Some(first.to_owned())
        } else {
            None
        }
    };
    let vol = if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        s[..2].to_owned()
    } else if let Some(rest) = s.strip_prefix(r"\\") {
        let mut it = rest.split(BS);
        let first = it.next()?;
        if first == "?" || first == "." {
            let second = it.next()?;
            device(second, &mut it)?
        } else {
            // UNC のサーバとシェア
            format!("UNC{BS}{first}{BS}{}", it.next()?)
        }
    } else {
        // `canonical_key` が verbatim の接頭辞を外した装置の形 (`GLOBALROOT\Device\…`・`Volume{…}\…`)
        let mut it = s.split(BS);
        let first = it.next()?;
        device(first, &mut it)?
    };
    // 置換文字 (UTF-16 として正しくない名前) を含むボリュームは比べない
    (!vol.is_empty() && !vol.contains('\u{FFFD}')).then(|| vol.to_ascii_uppercase())
}

/// ジョブの側で、調べたファイルの識別 `file_id` と等しい識別を持つ文書 (`known`) のうち、今は同じ識別でないもの (作り
/// 直された・消えた・`metadata` に失敗した) を返す (03 §12.7.6 の「古い識別の確かめ」)。文書のファイルが外で作り直される
/// と、文書の識別は古いまま残り、その番号を別のファイルが使うことがある (inode を使い回す Linux の FS)。主スレッドは
/// 返した文書の `file_id_unique` を下ろす (`demote_stale_ids`)。
pub(crate) fn stale_known_ids(
    fs: &dyn FileSystem,
    file_id: Option<FileId>,
    known: &[KnownFileId],
) -> Vec<KnownFileId> {
    let Some(id) = file_id else {
        return Vec::new();
    };
    known
        .iter()
        .filter(|k| k.file_id == id)
        .filter(|k| !fs.metadata(&k.path).is_ok_and(|m| m.file_id == Some(id)))
        .cloned()
        .collect()
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

/// ジョブの側で求める canonical のパス (03 §12.7.6 の 0 / 2) と、それが `path` 自身を canonicalize したものか
/// (`CanonicalSource::Direct`)。`path` を canonicalize できなければ (新規のファイル・名前を付けて保存の新しい保存先・
/// ダングリングのリンク・DOS の名前の無いボリュームの先)、親ディレクトリを canonicalize してファイル名を付けたもの
/// (`Fallback`)。どちらも取れなければ `None` (主スレッドが正規化したパスで代える)。在るファイルで `Direct` でないものは、
/// 呼び側が識別を使わない (`file_id_unique = false`。代わりのパスではボリュームの条件が効かないため)。
pub(crate) fn canonical_of(
    fs: &dyn FileSystem,
    path: &FsPath,
) -> (Option<FsPath>, CanonicalSource) {
    if let Ok(c) = fs.canonicalize(path) {
        return (Some(c), CanonicalSource::Direct);
    }
    let parent = path
        .as_path()
        .file_name()
        .zip(path.parent())
        .and_then(|(name, parent)| fs.canonicalize(&parent).ok().map(|dir| dir.join(name)));
    (parent, CanonicalSource::Fallback)
}

/// canonical のパスの出どころ (`canonical_of`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CanonicalSource {
    /// `path` 自身を canonicalize した。
    Direct,
    /// 親で代えた、または取れなかった (在るファイルなら識別を使わない)。
    Fallback,
}

impl crate::editor::state::EditorState {
    /// 開くジョブ・名前を付けて保存の照合のジョブに渡す、文書の識別の一覧 (03 §12.7.6 の「古い識別の確かめ」)。信頼できる
    /// 識別とパスを持つ文書だけ。`except` の文書 (これから開く文書自身) は除く。
    pub(crate) fn known_file_ids(&self, except: Option<eve_view::DocumentId>) -> Vec<KnownFileId> {
        self.docs
            .documents
            .iter()
            .filter(|(id, _)| Some(**id) != except)
            .filter_map(|(_, d)| {
                // 識別を先に確かめ、識別のある文書だけパスを複製する
                let ident = d.identity_ref();
                let file_id = ident.file_id.filter(|_| ident.file_id_unique)?;
                Some(KnownFileId {
                    path: d.path.as_ref()?.clone(),
                    file_id,
                })
            })
            .collect()
    }

    /// ジョブが古いと確かめた文書の識別 (`stale`) を、同じファイルの判定に使わないようにする (03 §12.7.6 の「古い識別の
    /// 確かめ」)。パスと識別がジョブに渡したときのままの文書だけを下ろす (その後に保存・再読込で直った文書は触らない)。
    pub(crate) fn demote_stale_ids(&mut self, stale: &[KnownFileId]) {
        if stale.is_empty() {
            return;
        }
        for d in self.docs.documents.values_mut() {
            let file_id = d.identity_ref().file_id;
            if stale
                .iter()
                .any(|k| Some(k.file_id) == file_id && d.path.as_ref() == Some(&k.path))
            {
                d.file_id_unique = false;
            }
        }
    }

    /// 開くジョブの識別 (`JobResult::Identity`、03 §9.2 手順 2b・§12.7.6 の 1) を受けた。新しく開く読込 (`OpenKind::Open`)
    /// で、ほかの文書 (読込中・確認待ちを含む) と同じファイルなら、新しい文書を閉じて読込を取り消し、既存の文書を開いた
    /// 時点のペインに出す。違えば (再読込・開き直しでも) 文書に識別を覚える。
    pub(crate) fn on_open_identity(
        &mut self,
        job: eve_view::JobId,
        mut ident: FileIdentity,
        stale: &[KnownFileId],
    ) {
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
        let (doc, replace, kind, opened) = (*doc, *replace, *kind, path.clone());
        // canonicalize できなかったときは正規化したパスで代え、識別は使わない (代わりのパスではボリュームの条件が効かない。
        // 03 §12.7.6 の 0)
        if ident.canonical.is_none() {
            ident.canonical = Some(opened.clone());
            ident.file_id_unique = false;
        }
        // 識別が古いと確かめた文書は、以後は識別で比べない
        self.demote_stale_ids(stale);
        if kind == super::open::OpenKind::Open
            && let Some(existing) = self.document_same_as(&ident, Some(doc))
        {
            self.open_already_open(job, doc, existing, replace, &opened);
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
        opened: &FsPath,
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
        let existing_path = self.document(existing).and_then(|d| d.path.clone());
        if let Some(other) = existing_path.filter(|p| p != opened) {
            // 打ったのと書き方の違うパスの文書に切り替える: どの文書として開いているかを示す (03 §9.2 手順 2b)
            self.show_message(
                eve_view::MessageLevel::Info,
                crate::ui_model::keys::INFO_OPEN_SAME_FILE,
                &[("path", &opened.display()), ("other", &other.display())],
            );
        } else if replace.is_some_and(|r| r.doc == existing) {
            self.message(
                eve_view::MessageLevel::Info,
                crate::ui_model::keys::INFO_OPEN_ALREADY_OPEN,
                &opened.display(),
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
    fn 識別が等しくてもボリュームが違えば同じとしない() {
        // ディスクの複製・バックアップの VHDX・シャドウコピー: シリアル番号と識別がそのまま重なる (05 §5.4)
        let a = ident(id(5), true, Some(r"C:\w\a.txt"));
        let b = ident(id(5), true, Some(r"E:\w\a.txt"));
        assert!(!a.same_file(&b));
        let shadow = ident(
            id(5),
            true,
            Some(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy3\w\a.txt"),
        );
        assert!(!a.same_file(&shadow));
        let unc = ident(id(5), true, Some(r"\\srv\share\w\a.txt"));
        assert!(!a.same_file(&unc));
        // 同じボリュームのハードリンクは同じ (ドライブ文字・サーバ名の大文字小文字は畳む)
        let hard = ident(id(5), true, Some(r"c:\other\hard.txt"));
        assert!(a.same_file(&hard));
        let unc2 = ident(id(5), true, Some(r"\\?\UNC\SRV\share\x\h.txt"));
        assert!(unc.same_file(&unc2));
    }

    #[test]
    fn ボリュームの部分() {
        let v = |s: &str| volume_of(&FsPath::from(s));
        assert_eq!(v(r"C:\a").as_deref(), Some("C:"));
        assert_eq!(v(r"\\?\d:\a").as_deref(), Some("D:"));
        assert_eq!(v(r"\\srv\Sh\a").as_deref(), Some(r"UNC\SRV\SH"));
        assert_eq!(v(r"\\?\UNC\srv\sh\a").as_deref(), Some(r"UNC\SRV\SH"));
        assert_eq!(
            v(r"\\?\GLOBALROOT\Device\HarddiskVolumeShadowCopy1\a").as_deref(),
            Some(r"GLOBALROOT\DEVICE\HARDDISKVOLUMESHADOWCOPY1")
        );
        assert_eq!(
            v(r"\\?\Volume{0000-11}\a").as_deref(),
            Some("VOLUME{0000-11}")
        );
        assert_eq!(v("/w/a.txt").as_deref(), Some(""), "Unix は条件を置かない");
        assert_eq!(v("relative/a.txt"), None);
        assert_eq!(
            v("\u{3042}\u{3044}\u{3046}\\x"),
            None,
            "多バイトの文字で落ちない"
        );
        // 識別があってもボリュームが分からなければ識別では同じとしない
        let a = ident(id(5), true, Some("rel/a.txt"));
        let b = ident(id(5), true, Some("rel/b.txt"));
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
