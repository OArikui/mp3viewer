//! MP3 Viewer — MP3メタデータの参照・編集ツール
//!
//! v0.1.0 の機能:
//!  1. メタデータの参照・編集
//!  2. 指定ディレクトリ内のmp3ファイル取得(再帰)
//!  3. デフォルトディレクトリの設定
//!  4. ディレクトリ構成によるアルバム/アーティスト設定とメタデータ書き込み
//!  5. ジャンル候補(説明付き)JSONの作成
//!  6. ジャンル候補のインポート/エクスポート
//!
//! v0.2.0 で追加:
//!  7. ファイル一覧でのクイックルック(ジャンル・アーティスト・アルバム・リリース年)
//!  8. ジャンル/アルバム/追加日/リリース年/アーティスト/曲名頭文字によるグループ表示・並び替え
//!  9. 曲名・アーティスト・アルバム・ジャンル・ファイル名を対象とした検索機能
//!
//! v0.3.0 で追加:
//! 10. ファイル一覧をテーブル表示に変更(列ごとにメタデータを表示)
//! 11. アーティスト・ファイル(メディア)・アルバムの埋め込み画像をサムネイル表示
//!     (ID3 APICのPictureType: Artist/LeadArtist・Media・CoverFrontをそれぞれ使用)

use chrono::{DateTime, Local};
use eframe::egui;
use egui_extras::{Column, TableBuilder};
use lofty::config::WriteOptions;
use lofty::picture::PictureType;
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::time::SystemTime;
use walkdir::WalkDir;

const APP_NAME: &str = "MP3 Viewer";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const GENRE_FORMAT_VERSION: u32 = 1;
const THUMB_MAX_PX: u32 = 64;
const THUMB_DISPLAY_PX: f32 = 32.0;

// ---------------------------------------------------------------------------
// 設定 / ジャンル候補の保存
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Default, Clone)]
struct Config {
    default_dir: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct Genre {
    name: String,
    #[serde(default)]
    description: String,
}

fn default_genre_version() -> u32 {
    GENRE_FORMAT_VERSION
}

#[derive(Serialize, Deserialize)]
struct GenreFile {
    #[serde(default = "default_genre_version")]
    version: u32,
    genres: Vec<Genre>,
}

/// インポート時は `{"version":1,"genres":[...]}` と `[...]` の両方を受け付ける
#[derive(Deserialize)]
#[serde(untagged)]
enum GenreImport {
    File(GenreFile),
    List(Vec<Genre>),
}

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mp3viewer")
}

fn config_path() -> PathBuf {
    config_dir().join("config.json")
}

fn genres_path() -> PathBuf {
    config_dir().join("genres.json")
}

fn load_config() -> Config {
    fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_config(cfg: &Config) -> Result<(), String> {
    fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    fs::write(config_path(), json).map_err(|e| e.to_string())
}

fn default_genres() -> Vec<Genre> {
    let g = |n: &str, d: &str| Genre {
        name: n.to_string(),
        description: d.to_string(),
    };
    vec![
        g("Rock", "ギター・ベース・ドラムを中心としたバンド編成の音楽"),
        g("Pop", "幅広い層に向けた親しみやすいメロディ中心の音楽"),
        g("J-Pop", "日本のポピュラー音楽"),
        g("Jazz", "即興演奏とスウィング感を特徴とする音楽"),
        g("Classical", "クラシック音楽。交響曲・協奏曲・室内楽など"),
        g("Hip-Hop", "ラップとビートを中心とした音楽"),
        g("R&B", "リズム&ブルース。ソウルフルなボーカルが特徴"),
        g("Electronic", "シンセサイザーやコンピュータで制作された音楽"),
        g("Folk", "民謡・フォーク。アコースティック中心の素朴な音楽"),
        g("Metal", "歪んだギターと激しいリズムが特徴のロックの派生"),
        g("Blues", "12小節形式とブルーノートを特徴とする音楽"),
        g("Soundtrack", "映画・アニメ・ゲームなどのサウンドトラック"),
    ]
}

fn write_genres(genres: &[Genre]) -> Result<(), String> {
    fs::create_dir_all(config_dir()).map_err(|e| e.to_string())?;
    let json = genres_json(genres)?;
    fs::write(genres_path(), json).map_err(|e| e.to_string())
}

fn genres_json(genres: &[Genre]) -> Result<String, String> {
    let file = GenreFile {
        version: GENRE_FORMAT_VERSION,
        genres: genres.to_vec(),
    };
    serde_json::to_string_pretty(&file).map_err(|e| e.to_string())
}

/// genres.json が無ければ既定内容で作成する
fn load_or_create_genres() -> Vec<Genre> {
    if let Ok(text) = fs::read_to_string(genres_path()) {
        if let Ok(parsed) = serde_json::from_str::<GenreImport>(&text) {
            return match parsed {
                GenreImport::File(f) => f.genres,
                GenreImport::List(l) => l,
            };
        }
    }
    let defaults = default_genres();
    let _ = write_genres(&defaults);
    defaults
}

// ---------------------------------------------------------------------------
// メタデータ入出力 (lofty)
// ---------------------------------------------------------------------------

#[derive(Clone, Default, PartialEq)]
struct Meta {
    title: String,
    artist: String,
    album: String,
    album_artist: String,
    genre: String,
    year: String,
    track: String,
}

/// 1ファイル分の読み込み結果(メタデータ・再生情報・埋め込み画像)
struct Loaded {
    meta: Meta,
    info: String,
    /// アルバム(表紙)画像。CoverFrontが無ければ埋め込み画像のうち最初のものを流用
    album_pic: Option<Vec<u8>>,
    /// アーティスト画像(PictureType::Artist / LeadArtist)
    artist_pic: Option<Vec<u8>>,
    /// メディア(盤面など)画像(PictureType::Media)
    media_pic: Option<Vec<u8>>,
}

fn read_meta(path: &Path) -> Result<Loaded, String> {
    let tf = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    let props = tf.properties();
    let secs = props.duration().as_secs();
    let mut info = format!("長さ {}:{:02}", secs / 60, secs % 60);
    if let Some(br) = props.audio_bitrate() {
        info.push_str(&format!(" / {} kbps", br));
    }
    if let Some(sr) = props.sample_rate() {
        info.push_str(&format!(" / {} Hz", sr));
    }

    let mut m = Meta::default();
    let mut album_pic: Option<Vec<u8>> = None;
    let mut artist_pic: Option<Vec<u8>> = None;
    let mut media_pic: Option<Vec<u8>> = None;

    if let Some(tag) = tf.primary_tag().or_else(|| tf.first_tag()) {
        m.title = tag.title().map(|s| s.to_string()).unwrap_or_default();
        m.artist = tag.artist().map(|s| s.to_string()).unwrap_or_default();
        m.album = tag.album().map(|s| s.to_string()).unwrap_or_default();
        m.genre = tag.genre().map(|s| s.to_string()).unwrap_or_default();
        m.album_artist = tag
            .get_string(&ItemKey::AlbumArtist)
            .unwrap_or_default()
            .to_string();
        m.year = tag.year().map(|y| y.to_string()).unwrap_or_default();
        m.track = tag.track().map(|t| t.to_string()).unwrap_or_default();

        for pic in tag.pictures() {
            match pic.pic_type() {
                PictureType::CoverFront => {
                    if album_pic.is_none() {
                        album_pic = Some(pic.data().to_vec());
                    }
                }
                PictureType::Artist | PictureType::LeadArtist => {
                    if artist_pic.is_none() {
                        artist_pic = Some(pic.data().to_vec());
                    }
                }
                PictureType::Media => {
                    if media_pic.is_none() {
                        media_pic = Some(pic.data().to_vec());
                    }
                }
                _ => {}
            }
        }
        if album_pic.is_none() {
            if let Some(p) = tag.pictures().first() {
                album_pic = Some(p.data().to_vec());
            }
        }
    }

    Ok(Loaded {
        meta: m,
        info,
        album_pic,
        artist_pic,
        media_pic,
    })
}

/// タグを読み込み、`f` で書き換えて保存する(タグが無ければ新規作成)
fn patch_tag<F: FnOnce(&mut Tag)>(path: &Path, f: F) -> Result<(), String> {
    let mut tf = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    if tf.primary_tag().is_none() {
        let tag = Tag::new(tf.primary_tag_type());
        tf.insert_tag(tag);
    }
    {
        let tag = tf
            .primary_tag_mut()
            .ok_or_else(|| "タグを作成できませんでした".to_string())?;
        f(tag);
    }
    tf.save_to_path(path, WriteOptions::default())
        .map_err(|e| e.to_string())
}

fn parse_opt_u32(s: &str, label: &str) -> Result<Option<u32>, String> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    s.parse::<u32>()
        .map(Some)
        .map_err(|_| format!("{label}は数値で入力してください: {s}"))
}

fn write_meta(path: &Path, m: &Meta) -> Result<(), String> {
    let year = parse_opt_u32(&m.year, "年")?;
    let track = parse_opt_u32(&m.track, "トラック番号")?;
    let m = m.clone();
    patch_tag(path, move |t| {
        if m.title.is_empty() {
            t.remove_title();
        } else {
            t.set_title(m.title);
        }
        if m.artist.is_empty() {
            t.remove_artist();
        } else {
            t.set_artist(m.artist);
        }
        if m.album.is_empty() {
            t.remove_album();
        } else {
            t.set_album(m.album);
        }
        if m.genre.is_empty() {
            t.remove_genre();
        } else {
            t.set_genre(m.genre);
        }
        if m.album_artist.is_empty() {
            t.remove_key(&ItemKey::AlbumArtist);
        } else {
            t.insert_text(ItemKey::AlbumArtist, m.album_artist);
        }
        match year {
            Some(y) => t.set_year(y),
            None => t.remove_year(),
        }
        match track {
            Some(n) => t.set_track(n),
            None => t.remove_track(),
        }
    })
}

// ---------------------------------------------------------------------------
// ディレクトリ走査 / ディレクトリ構成からの推定
// ---------------------------------------------------------------------------

fn scan_mp3(root: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.path()
                .extension()
                .map(|x| x.eq_ignore_ascii_case("mp3"))
                .unwrap_or(false)
        })
        .map(|e| e.into_path())
        .collect();
    files.sort();
    files
}

/// 規則: ルート/アーティスト/アルバム/曲.mp3
/// - 親ディレクトリが2階層以上: (末尾から2番目=アーティスト, 末尾=アルバム)
/// - 1階層のみ: アルバムのみ
/// - ルート直下: どちらも無し
fn derive_from_dir(root: &Path, file: &Path) -> (Option<String>, Option<String>) {
    let Ok(rel) = file.strip_prefix(root) else {
        return (None, None);
    };
    let comps: Vec<String> = rel
        .parent()
        .map(|p| {
            p.components()
                .filter_map(|c| match c {
                    Component::Normal(s) => Some(s.to_string_lossy().to_string()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    match comps.len() {
        0 => (None, None),
        1 => (None, Some(comps[0].clone())),
        n => (Some(comps[n - 2].clone()), Some(comps[n - 1].clone())),
    }
}

fn file_added_time(path: &Path) -> SystemTime {
    fs::metadata(path)
        .and_then(|m| m.created().or_else(|_| m.modified()))
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn format_added(t: SystemTime) -> String {
    let dt: DateTime<Local> = t.into();
    dt.format("%Y-%m-%d").to_string()
}

/// 埋め込み画像のバイト列をデコードし、正方形に収まるサムネイルへ縮小する
fn decode_thumb(bytes: &[u8], max: u32) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.thumbnail(max, max).to_rgba8();
    let w = img.width() as usize;
    let h = img.height() as usize;
    if w == 0 || h == 0 {
        return None;
    }
    Some(egui::ColorImage::from_rgba_unmultiplied([w, h], img.as_raw()))
}

// ---------------------------------------------------------------------------
// ファイル一覧のエントリ(クイックルック用にメタデータをキャッシュ)
// ---------------------------------------------------------------------------

struct FileEntry {
    path: PathBuf,
    rel: String,
    added: SystemTime,
    meta: Meta,
    info: String,
    album_pic: Option<Vec<u8>>,
    artist_pic: Option<Vec<u8>>,
    media_pic: Option<Vec<u8>>,
    /// 検索用に小文字化した rel/title/artist/album/genre の連結文字列
    search_blob: String,
}

fn build_search_blob(rel: &str, m: &Meta) -> String {
    format!("{} {} {} {} {}", rel, m.title, m.artist, m.album, m.genre).to_lowercase()
}

fn build_entry(root: &Path, path: PathBuf) -> FileEntry {
    let rel = path
        .strip_prefix(root)
        .unwrap_or(&path)
        .to_string_lossy()
        .to_string();
    let loaded = read_meta(&path).unwrap_or_else(|e| Loaded {
        meta: Meta::default(),
        info: format!("(読み込みエラー: {e})"),
        album_pic: None,
        artist_pic: None,
        media_pic: None,
    });
    let added = file_added_time(&path);
    let search_blob = build_search_blob(&rel, &loaded.meta);
    FileEntry {
        path,
        rel,
        added,
        meta: loaded.meta,
        info: loaded.info,
        album_pic: loaded.album_pic,
        artist_pic: loaded.artist_pic,
        media_pic: loaded.media_pic,
        search_blob,
    }
}

fn dash_if_empty(s: &str) -> &str {
    let t = s.trim();
    if t.is_empty() {
        "-"
    } else {
        t
    }
}

fn empty_or(s: &str, fallback: &str) -> String {
    let t = s.trim();
    if t.is_empty() {
        fallback.to_string()
    } else {
        t.to_string()
    }
}

/// テーブルの曲名セルに表示するタイトル(未設定ならファイル名を代用)
fn display_title(e: &FileEntry) -> String {
    if e.meta.title.trim().is_empty() {
        Path::new(&e.rel)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| e.rel.clone())
    } else {
        e.meta.title.clone()
    }
}

fn title_initial(e: &FileEntry) -> String {
    let base = if !e.meta.title.trim().is_empty() {
        e.meta.title.trim().to_string()
    } else {
        Path::new(&e.rel)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    };
    match base.chars().find(|c| c.is_alphanumeric()) {
        Some(c) => c.to_uppercase().collect::<String>(),
        None => "#".to_string(),
    }
}

/// 並び替え・グループ表示のキー
#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    FileName,
    Genre,
    Artist,
    Album,
    AddedDate,
    ReleaseDate,
    TitleInitial,
}

impl SortKey {
    const ALL: [SortKey; 7] = [
        SortKey::FileName,
        SortKey::Genre,
        SortKey::Artist,
        SortKey::Album,
        SortKey::AddedDate,
        SortKey::ReleaseDate,
        SortKey::TitleInitial,
    ];

    fn label(self) -> &'static str {
        match self {
            SortKey::FileName => "ファイル名順(グループなし)",
            SortKey::Genre => "ジャンル別",
            SortKey::Artist => "アーティスト別",
            SortKey::Album => "アルバム別",
            SortKey::AddedDate => "追加日別",
            SortKey::ReleaseDate => "リリース年別",
            SortKey::TitleInitial => "曲名頭文字別",
        }
    }
}

fn group_label(e: &FileEntry, key: SortKey) -> String {
    match key {
        SortKey::FileName => String::new(),
        SortKey::Genre => empty_or(&e.meta.genre, "(ジャンル未設定)"),
        SortKey::Artist => empty_or(&e.meta.artist, "(アーティスト未設定)"),
        SortKey::Album => empty_or(&e.meta.album, "(アルバム未設定)"),
        SortKey::AddedDate => format_added(e.added),
        SortKey::ReleaseDate => empty_or(&e.meta.year, "(年不明)"),
        SortKey::TitleInitial => title_initial(e),
    }
}

/// サムネイルの種別(いずれもID3 APICのPictureTypeに対応)
#[derive(Clone, Copy, PartialEq, Eq)]
enum ThumbKind {
    Artist,
    Media,
    Album,
}

/// テーブル1行分の描画用データ(クロージャ内でselfを借用しないよう事前に構築する)
struct RowView {
    idx: usize,
    group_header: Option<String>,
    artist_tex: Option<egui::TextureHandle>,
    media_tex: Option<egui::TextureHandle>,
    album_tex: Option<egui::TextureHandle>,
    title: String,
    artist: String,
    album: String,
    genre: String,
    year: String,
}

fn show_thumb(ui: &mut egui::Ui, tex: &Option<egui::TextureHandle>) {
    match tex {
        Some(t) => {
            ui.add(
                egui::Image::new((t.id(), t.size_vec2()))
                    .fit_to_exact_size(egui::vec2(THUMB_DISPLAY_PX, THUMB_DISPLAY_PX)),
            );
        }
        None => {
            ui.weak("–");
        }
    }
}

// ---------------------------------------------------------------------------
// GUI
// ---------------------------------------------------------------------------

#[derive(PartialEq, Clone, Copy)]
enum Tab {
    Editor,
    Genres,
}

#[derive(PartialEq, Clone, Copy)]
enum BulkKind {
    Album,
    Artist,
}

#[derive(Clone, Copy)]
struct Pending {
    kind: BulkKind,
    all: bool,
}

struct App {
    cfg: Config,
    genres: Vec<Genre>,
    root: Option<PathBuf>,
    entries: Vec<FileEntry>,
    search: String,
    sort_key: SortKey,
    selected: Option<usize>,
    meta: Meta,
    orig: Meta,
    info: String,
    status: String,
    tab: Tab,
    new_genre: Genre,
    pending: Option<Pending>,
    also_album_artist: bool,
    show_about: bool,
    artist_thumbs: HashMap<usize, Option<egui::TextureHandle>>,
    media_thumbs: HashMap<usize, Option<egui::TextureHandle>>,
    album_thumbs: HashMap<usize, Option<egui::TextureHandle>>,
}

impl App {
    fn new() -> Self {
        let cfg = load_config();
        let genres = load_or_create_genres();
        let mut app = App {
            cfg: cfg.clone(),
            genres,
            root: None,
            entries: vec![],
            search: String::new(),
            sort_key: SortKey::FileName,
            selected: None,
            meta: Meta::default(),
            orig: Meta::default(),
            info: String::new(),
            status: "準備完了".to_string(),
            tab: Tab::Editor,
            new_genre: Genre::default(),
            pending: None,
            also_album_artist: true,
            show_about: false,
            artist_thumbs: HashMap::new(),
            media_thumbs: HashMap::new(),
            album_thumbs: HashMap::new(),
        };
        if let Some(d) = cfg.default_dir {
            let p = PathBuf::from(d);
            if p.is_dir() {
                app.open_dir(p);
            }
        }
        app
    }

    fn open_dir(&mut self, dir: PathBuf) {
        let paths = scan_mp3(&dir);
        self.entries = paths.into_iter().map(|p| build_entry(&dir, p)).collect();
        self.status = format!("{} 個のmp3を取得: {}", self.entries.len(), dir.display());
        self.root = Some(dir);
        self.selected = None;
        self.meta = Meta::default();
        self.orig = Meta::default();
        self.info.clear();
        self.artist_thumbs.clear();
        self.media_thumbs.clear();
        self.album_thumbs.clear();
    }

    fn select(&mut self, idx: usize) {
        let Some(e) = self.entries.get(idx) else {
            return;
        };
        self.meta = e.meta.clone();
        self.orig = e.meta.clone();
        self.info = e.info.clone();
        self.selected = Some(idx);
    }

    /// ディスクから再読込してキャッシュを更新する(保存/一括書き込み後に使用)
    fn refresh_entry(&mut self, idx: usize) {
        if let Some(e) = self.entries.get_mut(idx) {
            if let Ok(loaded) = read_meta(&e.path) {
                e.search_blob = build_search_blob(&e.rel, &loaded.meta);
                e.meta = loaded.meta;
                e.info = loaded.info;
                e.album_pic = loaded.album_pic;
                e.artist_pic = loaded.artist_pic;
                e.media_pic = loaded.media_pic;
            }
        }
        self.artist_thumbs.remove(&idx);
        self.media_thumbs.remove(&idx);
        self.album_thumbs.remove(&idx);
    }

    /// サムネイルテクスチャを取得する(初回のみデコードし、以降はキャッシュを返す)
    fn get_thumb(&mut self, ctx: &egui::Context, idx: usize, kind: ThumbKind) -> Option<egui::TextureHandle> {
        let kind_str = match kind {
            ThumbKind::Artist => "artist",
            ThumbKind::Media => "media",
            ThumbKind::Album => "album",
        };
        let cache = match kind {
            ThumbKind::Artist => &mut self.artist_thumbs,
            ThumbKind::Media => &mut self.media_thumbs,
            ThumbKind::Album => &mut self.album_thumbs,
        };
        if let Some(v) = cache.get(&idx) {
            return v.clone();
        }
        let bytes = match kind {
            ThumbKind::Artist => self.entries.get(idx).and_then(|e| e.artist_pic.as_ref()),
            ThumbKind::Media => self.entries.get(idx).and_then(|e| e.media_pic.as_ref()),
            ThumbKind::Album => self.entries.get(idx).and_then(|e| e.album_pic.as_ref()),
        };
        let tex = bytes.and_then(|b| decode_thumb(b, THUMB_MAX_PX)).map(|img| {
            ctx.load_texture(format!("thumb-{kind_str}-{idx}"), img, egui::TextureOptions::default())
        });
        let cache = match kind {
            ThumbKind::Artist => &mut self.artist_thumbs,
            ThumbKind::Media => &mut self.media_thumbs,
            ThumbKind::Album => &mut self.album_thumbs,
        };
        cache.insert(idx, tex.clone());
        tex
    }

    fn set_default_dir(&mut self) {
        let Some(root) = self.root.clone() else {
            self.status = "先にディレクトリを開いてください".to_string();
            return;
        };
        self.cfg.default_dir = Some(root.to_string_lossy().to_string());
        self.status = match save_config(&self.cfg) {
            Ok(_) => format!("デフォルトディレクトリを設定: {}", root.display()),
            Err(e) => format!("設定の保存に失敗: {e}"),
        };
    }

    fn save_current(&mut self) {
        let Some(idx) = self.selected else {
            return;
        };
        let Some(path) = self.entries.get(idx).map(|e| e.path.clone()) else {
            return;
        };
        match write_meta(&path, &self.meta) {
            Ok(_) => {
                self.refresh_entry(idx);
                if let Some(e) = self.entries.get(idx) {
                    self.orig = e.meta.clone();
                    self.info = e.info.clone();
                }
                self.status = format!("保存しました: {}", path.display());
            }
            Err(e) => self.status = format!("保存失敗: {e}"),
        }
    }

    fn run_bulk(&mut self, p: Pending) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let targets: Vec<usize> = if p.all {
            (0..self.entries.len()).collect()
        } else {
            self.selected.into_iter().collect()
        };
        let also = self.also_album_artist;
        let (mut ok, mut skipped, mut failed) = (0usize, 0usize, 0usize);
        for &i in &targets {
            let Some(path) = self.entries.get(i).map(|e| e.path.clone()) else {
                continue;
            };
            let (artist, album) = derive_from_dir(&root, &path);
            let value = match p.kind {
                BulkKind::Album => album,
                BulkKind::Artist => artist,
            };
            let Some(v) = value else {
                skipped += 1;
                continue;
            };
            let kind = p.kind;
            let r = patch_tag(&path, move |t| match kind {
                BulkKind::Album => t.set_album(v),
                BulkKind::Artist => {
                    t.set_artist(v.clone());
                    if also {
                        t.insert_text(ItemKey::AlbumArtist, v);
                    }
                }
            });
            match r {
                Ok(_) => {
                    ok += 1;
                    self.refresh_entry(i);
                }
                Err(_) => failed += 1,
            }
        }
        let label = match p.kind {
            BulkKind::Album => "アルバム",
            BulkKind::Artist => "アーティスト",
        };
        self.status = format!(
            "{label}書き込み完了: 成功 {ok} / スキップ(階層なし) {skipped} / 失敗 {failed}"
        );
        if let Some(i) = self.selected {
            self.select(i);
        }
    }

    fn save_genres(&mut self) {
        if let Err(e) = write_genres(&self.genres) {
            self.status = format!("ジャンル候補の保存に失敗: {e}");
        }
    }

    fn import_genres(&mut self, replace: bool) {
        let Some(p) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .pick_file()
        else {
            return;
        };
        let text = match fs::read_to_string(&p) {
            Ok(t) => t,
            Err(e) => {
                self.status = format!("読み込み失敗: {e}");
                return;
            }
        };
        let list = match serde_json::from_str::<GenreImport>(&text) {
            Ok(GenreImport::File(f)) => f.genres,
            Ok(GenreImport::List(l)) => l,
            Err(e) => {
                self.status = format!("JSONの形式が不正です: {e}");
                return;
            }
        };
        let list: Vec<Genre> = list
            .into_iter()
            .filter(|g| !g.name.trim().is_empty())
            .collect();
        let n = list.len();
        if replace {
            self.genres = list;
        } else {
            for g in list {
                if let Some(e) = self.genres.iter_mut().find(|x| x.name == g.name) {
                    e.description = g.description;
                } else {
                    self.genres.push(g);
                }
            }
        }
        self.save_genres();
        self.status = format!(
            "ジャンル候補をインポート({}): {} 件",
            if replace { "置換" } else { "マージ" },
            n
        );
    }

    fn export_genres(&mut self) {
        let Some(p) = rfd::FileDialog::new()
            .add_filter("JSON", &["json"])
            .set_file_name("genres.json")
            .save_file()
        else {
            return;
        };
        self.status = match genres_json(&self.genres)
            .and_then(|j| fs::write(&p, j).map_err(|e| e.to_string()))
        {
            Ok(_) => format!("エクスポートしました: {}", p.display()),
            Err(e) => format!("エクスポート失敗: {e}"),
        };
    }

    // ---- UI parts ----

    fn editor_ui(&mut self, ui: &mut egui::Ui) {
        let Some(sel) = self.selected else {
            ui.label("左のリストからmp3ファイルを選択してください");
            return;
        };
        let Some(path) = self.entries.get(sel).map(|e| e.path.clone()) else {
            return;
        };
        ui.heading(
            path.file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default(),
        );
        ui.weak(path.display().to_string());
        if !self.info.is_empty() {
            ui.label(self.info.clone());
        }
        ui.separator();

        egui::Grid::new("meta_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                text_row(ui, "タイトル", &mut self.meta.title);
                text_row(ui, "アーティスト", &mut self.meta.artist);
                text_row(ui, "アルバム", &mut self.meta.album);
                text_row(ui, "アルバムアーティスト", &mut self.meta.album_artist);
                ui.label("ジャンル");
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.meta.genre).desired_width(240.0));
                    egui::ComboBox::from_id_salt("genre_combo")
                        .selected_text("候補から選択")
                        .show_ui(ui, |ui| {
                            for g in &self.genres {
                                ui.selectable_value(&mut self.meta.genre, g.name.clone(), g.name.clone())
                                    .on_hover_text(g.description.clone());
                            }
                        });
                });
                ui.end_row();
                text_row(ui, "年", &mut self.meta.year);
                text_row(ui, "トラック番号", &mut self.meta.track);
            });

        if let Some(g) = self.genres.iter().find(|g| g.name == self.meta.genre) {
            if !g.description.is_empty() {
                ui.weak(format!("ジャンル説明: {}", g.description));
            }
        }

        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let dirty = self.meta != self.orig;
            if ui.add_enabled(dirty, egui::Button::new("保存")).clicked() {
                self.save_current();
            }
            if ui.add_enabled(dirty, egui::Button::new("元に戻す")).clicked() {
                self.meta = self.orig.clone();
            }
            if dirty {
                ui.colored_label(egui::Color32::from_rgb(200, 120, 0), "未保存の変更があります");
            }
        });

        // ---- ディレクトリ構成からの設定 ----
        ui.add_space(12.0);
        ui.separator();
        ui.strong("ディレクトリ構成からの設定");
        ui.weak("規則: ルート/アーティスト/アルバム/曲.mp3(ルート直下にアルバムフォルダのみの場合はアルバムのみ)");
        let (da, dal) = match &self.root {
            Some(r) => derive_from_dir(r, &path),
            None => (None, None),
        };
        ui.label(format!(
            "検出: アーティスト = {} / アルバム = {}",
            da.as_deref().unwrap_or("(なし)"),
            dal.as_deref().unwrap_or("(なし)")
        ));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(dal.is_some(), egui::Button::new("アルバムをフォームに反映"))
                .clicked()
            {
                if let Some(a) = &dal {
                    self.meta.album = a.clone();
                }
            }
            if ui
                .add_enabled(da.is_some(), egui::Button::new("アーティストをフォームに反映"))
                .clicked()
            {
                if let Some(a) = &da {
                    self.meta.artist = a.clone();
                }
            }
        });
        ui.checkbox(
            &mut self.also_album_artist,
            "アーティスト書き込み時にアルバムアーティストも設定する",
        );
        ui.horizontal(|ui| {
            if ui.button("アルバム書き込み(選択ファイル)").clicked() {
                self.pending = Some(Pending { kind: BulkKind::Album, all: false });
            }
            if ui.button("アルバム書き込み(全ファイル)").clicked() {
                self.pending = Some(Pending { kind: BulkKind::Album, all: true });
            }
        });
        ui.horizontal(|ui| {
            if ui.button("アーティスト書き込み(選択ファイル)").clicked() {
                self.pending = Some(Pending { kind: BulkKind::Artist, all: false });
            }
            if ui.button("アーティスト書き込み(全ファイル)").clicked() {
                self.pending = Some(Pending { kind: BulkKind::Artist, all: true });
            }
        });
    }

    fn genres_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        ui.horizontal(|ui| {
            if ui.button("インポート(マージ)").clicked() {
                self.import_genres(false);
            }
            if ui.button("インポート(置換)").clicked() {
                self.import_genres(true);
            }
            if ui.button("エクスポート").clicked() {
                self.export_genres();
            }
        });
        ui.weak(format!("保存先: {}", genres_path().display()));
        ui.separator();

        ui.strong("新規追加");
        ui.horizontal(|ui| {
            ui.label("名前");
            ui.add(egui::TextEdit::singleline(&mut self.new_genre.name).desired_width(160.0));
            ui.label("説明");
            ui.add(egui::TextEdit::singleline(&mut self.new_genre.description).desired_width(320.0));
            let ok = !self.new_genre.name.trim().is_empty()
                && !self.genres.iter().any(|g| g.name == self.new_genre.name.trim());
            if ui.add_enabled(ok, egui::Button::new("追加")).clicked() {
                let g = Genre {
                    name: self.new_genre.name.trim().to_string(),
                    description: self.new_genre.description.clone(),
                };
                self.genres.push(g);
                self.new_genre = Genre::default();
                changed = true;
            }
        });
        ui.separator();

        let mut remove: Option<usize> = None;
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            egui::Grid::new("genre_grid")
                .num_columns(3)
                .spacing([10.0, 6.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.strong("名前");
                    ui.strong("説明");
                    ui.label("");
                    ui.end_row();
                    for (i, g) in self.genres.iter_mut().enumerate() {
                        changed |= ui
                            .add(egui::TextEdit::singleline(&mut g.name).desired_width(160.0))
                            .changed();
                        changed |= ui
                            .add(egui::TextEdit::singleline(&mut g.description).desired_width(420.0))
                            .changed();
                        if ui.button("削除").clicked() {
                            remove = Some(i);
                        }
                        ui.end_row();
                    }
                });
        });
        if let Some(i) = remove {
            self.genres.remove(i);
            changed = true;
        }
        if changed {
            self.save_genres();
        }
    }
}

fn text_row(ui: &mut egui::Ui, label: &str, val: &mut String) {
    ui.label(label);
    ui.add(egui::TextEdit::singleline(val).desired_width(f32::INFINITY));
    ui.end_row();
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ---- 上部バー ----
        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("ディレクトリを開く").clicked() {
                    if let Some(d) = rfd::FileDialog::new().pick_folder() {
                        self.open_dir(d);
                    }
                }
                if ui.button("再読込").clicked() {
                    if let Some(r) = self.root.clone() {
                        self.open_dir(r);
                    }
                }
                if ui.button("デフォルトに設定").clicked() {
                    self.set_default_dir();
                }
                let default_ok = self
                    .cfg
                    .default_dir
                    .as_ref()
                    .map(|d| Path::new(d).is_dir())
                    .unwrap_or(false);
                if ui
                    .add_enabled(default_ok, egui::Button::new("デフォルトを開く"))
                    .clicked()
                {
                    if let Some(d) = self.cfg.default_dir.clone() {
                        self.open_dir(PathBuf::from(d));
                    }
                }
                ui.separator();
                match &self.root {
                    Some(r) => ui.label(r.display().to_string()),
                    None => ui.weak("(ディレクトリ未選択)"),
                };
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("バージョン情報").clicked() {
                        self.show_about = true;
                    }
                });
            });
        });

        // ---- 下部ステータスバー ----
        egui::TopBottomPanel::bottom("status").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label(self.status.clone());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.weak(format!("{APP_NAME} v{APP_VERSION}"));
                });
            });
        });

        // ---- 左: ファイル一覧(テーブル表示・サムネイル・検索・グループ表示) ----
        let mut clicked: Option<usize> = None;
        egui::SidePanel::left("files")
            .resizable(true)
            .default_width(640.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("検索");
                    ui.text_edit_singleline(&mut self.search);
                });
                ui.weak("曲名・アーティスト・アルバム・ジャンル・ファイル名が対象です");

                egui::ComboBox::from_label("表示順 / グループ")
                    .selected_text(self.sort_key.label())
                    .show_ui(ui, |ui| {
                        for k in SortKey::ALL {
                            ui.selectable_value(&mut self.sort_key, k, k.label());
                        }
                    });

                // ---- 表示対象の絞り込み・並び替え ----
                let q = self.search.trim().to_lowercase();
                let key = self.sort_key;
                let mut visible: Vec<usize> = (0..self.entries.len())
                    .filter(|&i| q.is_empty() || self.entries[i].search_blob.contains(&q))
                    .collect();
                if key == SortKey::FileName {
                    visible.sort_by(|&a, &b| self.entries[a].rel.cmp(&self.entries[b].rel));
                } else {
                    visible.sort_by(|&a, &b| {
                        let la = group_label(&self.entries[a], key);
                        let lb = group_label(&self.entries[b], key);
                        la.cmp(&lb).then_with(|| self.entries[a].rel.cmp(&self.entries[b].rel))
                    });
                }
                ui.weak(format!("{} / {} 件", visible.len(), self.entries.len()));
                ui.separator();

                // ---- テーブル行の描画用データを事前に構築(サムネイル読込を含む) ----
                let render_ctx = ui.ctx().clone();
                let mut rows: Vec<RowView> = Vec::with_capacity(visible.len());
                let mut last_label: Option<String> = None;
                for &i in &visible {
                    let mut header = None;
                    if key != SortKey::FileName {
                        let lbl = group_label(&self.entries[i], key);
                        if last_label.as_ref() != Some(&lbl) {
                            header = Some(lbl.clone());
                            last_label = Some(lbl);
                        }
                    }
                    let artist_tex = self.get_thumb(&render_ctx, i, ThumbKind::Artist);
                    let media_tex = self.get_thumb(&render_ctx, i, ThumbKind::Media);
                    let album_tex = self.get_thumb(&render_ctx, i, ThumbKind::Album);
                    let e = &self.entries[i];
                    rows.push(RowView {
                        idx: i,
                        group_header: header,
                        artist_tex,
                        media_tex,
                        album_tex,
                        title: display_title(e),
                        artist: dash_if_empty(&e.meta.artist).to_string(),
                        album: dash_if_empty(&e.meta.album).to_string(),
                        genre: dash_if_empty(&e.meta.genre).to_string(),
                        year: dash_if_empty(&e.meta.year).to_string(),
                    });
                }
                let selected = self.selected;

                // ---- テーブル本体(この中では self を参照しない: rows/selected/clicked のみ使用) ----
                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                    .column(Column::exact(THUMB_DISPLAY_PX + 6.0))
                    .column(Column::exact(THUMB_DISPLAY_PX + 6.0))
                    .column(Column::exact(THUMB_DISPLAY_PX + 6.0))
                    .column(Column::initial(160.0).at_least(80.0))
                    .column(Column::initial(120.0).at_least(60.0))
                    .column(Column::initial(120.0).at_least(60.0))
                    .column(Column::initial(90.0).at_least(50.0))
                    .column(Column::initial(50.0).at_least(40.0))
                    .header(22.0, |mut header| {
                        header.col(|ui| { ui.strong("アーティスト"); });
                        header.col(|ui| { ui.strong("ファイル"); });
                        header.col(|ui| { ui.strong("アルバム"); });
                        header.col(|ui| { ui.strong("曲名"); });
                        header.col(|ui| { ui.strong("アーティスト"); });
                        header.col(|ui| { ui.strong("アルバム"); });
                        header.col(|ui| { ui.strong("ジャンル"); });
                        header.col(|ui| { ui.strong("年"); });
                    })
                    .body(|mut body| {
                        for r in &rows {
                            if let Some(lbl) = &r.group_header {
                                body.row(20.0, |mut row| {
                                    row.col(|ui| { ui.strong(lbl.as_str()); });
                                    row.col(|_ui| {});
                                    row.col(|_ui| {});
                                    row.col(|_ui| {});
                                    row.col(|_ui| {});
                                    row.col(|_ui| {});
                                    row.col(|_ui| {});
                                    row.col(|_ui| {});
                                });
                            }
                            body.row(THUMB_DISPLAY_PX + 6.0, |mut row| {
                                row.col(|ui| show_thumb(ui, &r.artist_tex));
                                row.col(|ui| show_thumb(ui, &r.media_tex));
                                row.col(|ui| show_thumb(ui, &r.album_tex));
                                row.col(|ui| {
                                    if ui
                                        .selectable_label(selected == Some(r.idx), r.title.as_str())
                                        .clicked()
                                    {
                                        clicked = Some(r.idx);
                                    }
                                });
                                row.col(|ui| { ui.label(r.artist.as_str()); });
                                row.col(|ui| { ui.label(r.album.as_str()); });
                                row.col(|ui| { ui.label(r.genre.as_str()); });
                                row.col(|ui| { ui.label(r.year.as_str()); });
                            });
                        }
                        if rows.is_empty() {
                            body.row(20.0, |mut row| {
                                row.col(|ui| { ui.weak("該当するファイルがありません"); });
                                row.col(|_ui| {});
                                row.col(|_ui| {});
                                row.col(|_ui| {});
                                row.col(|_ui| {});
                                row.col(|_ui| {});
                                row.col(|_ui| {});
                                row.col(|_ui| {});
                            });
                        }
                    });
            });
        if let Some(i) = clicked {
            self.select(i);
        }

        // ---- 中央 ----
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Editor, "メタデータ");
                ui.selectable_value(&mut self.tab, Tab::Genres, "ジャンル候補");
            });
            ui.separator();
            match self.tab {
                Tab::Editor => self.editor_ui(ui),
                Tab::Genres => self.genres_ui(ui),
            }
        });

        // ---- 一括書き込みの確認 ----
        if let Some(p) = self.pending {
            let n = if p.all {
                self.entries.len()
            } else if self.selected.is_some() {
                1
            } else {
                0
            };
            let label = match p.kind {
                BulkKind::Album => "アルバム",
                BulkKind::Artist => "アーティスト",
            };
            let mut decision: Option<bool> = None;
            egui::Window::new("書き込み確認")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(format!(
                        "{n} 個のファイルにディレクトリ構成由来の{label}を書き込みます。\n未保存のフォーム編集は破棄されます。よろしいですか?"
                    ));
                    ui.horizontal(|ui| {
                        if ui.button("実行").clicked() {
                            decision = Some(true);
                        }
                        if ui.button("キャンセル").clicked() {
                            decision = Some(false);
                        }
                    });
                });
            match decision {
                Some(true) => {
                    self.pending = None;
                    self.run_bulk(p);
                }
                Some(false) => self.pending = None,
                None => {}
            }
        }

        // ---- バージョン情報 ----
        if self.show_about {
            let mut open = true;
            egui::Window::new("バージョン情報")
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.heading(APP_NAME);
                    ui.label(format!("バージョン: {APP_VERSION}"));
                    ui.label(format!("ジャンルJSON形式: v{GENRE_FORMAT_VERSION}"));
                    ui.weak(format!("設定フォルダ: {}", config_dir().display()));
                });
            if !open {
                self.show_about = false;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// フォント(日本語表示用) / エントリポイント
// ---------------------------------------------------------------------------

fn setup_fonts(ctx: &egui::Context) {
    let candidates = [
        // Windows
        "C:\\Windows\\Fonts\\YuGothM.ttc",
        "C:\\Windows\\Fonts\\meiryo.ttc",
        "C:\\Windows\\Fonts\\msgothic.ttc",
        // macOS
        "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
    ];
    for p in candidates {
        if let Ok(bytes) = fs::read(p) {
            let mut fonts = egui::FontDefinitions::default();
            fonts
                .font_data
                .insert("jp".to_owned(), egui::FontData::from_owned(bytes));
            for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(fam).or_default().push("jp".to_owned());
            }
            ctx.set_fonts(fonts);
            return;
        }
    }
}

fn main() -> eframe::Result<()> {
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("{APP_NAME} {APP_VERSION}");
        return Ok(());
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 780.0])
            .with_title(format!("{APP_NAME} v{APP_VERSION}")),
        ..Default::default()
    };
    eframe::run_native(
        APP_NAME,
        options,
        Box::new(|cc| {
            setup_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new()))
        }),
    )
}
