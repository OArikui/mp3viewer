//! MP3 Viewer — MP3メタデータの参照・編集ツール
//!
//! 機能:
//!  1. メタデータの参照・編集
//!  2. 指定ディレクトリ内のmp3ファイル取得(再帰)
//!  3. デフォルトディレクトリの設定
//!  4. ディレクトリ構成によるアルバム/アーティスト設定とメタデータ書き込み
//!  5. ジャンル候補(説明付き)JSONの作成
//!  6. ジャンル候補のインポート/エクスポート

use eframe::egui;
use lofty::config::WriteOptions;
use lofty::prelude::*;
use lofty::tag::{ItemKey, Tag};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use walkdir::WalkDir;

const APP_NAME: &str = "MP3 Viewer";
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const GENRE_FORMAT_VERSION: u32 = 1;

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

/// genres.json が無ければ既定内容で作成する(⑤)
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

fn read_meta(path: &Path) -> Result<(Meta, String), String> {
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
    }
    Ok((m, info))
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
    files: Vec<PathBuf>,
    /// (表示用相対パス, 小文字化した検索用文字列)
    labels: Vec<(String, String)>,
    filter: String,
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
}

impl App {
    fn new() -> Self {
        let cfg = load_config();
        let genres = load_or_create_genres();
        let mut app = App {
            cfg: cfg.clone(),
            genres,
            root: None,
            files: vec![],
            labels: vec![],
            filter: String::new(),
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
        self.files = scan_mp3(&dir);
        self.labels = self
            .files
            .iter()
            .map(|f| {
                let rel = f
                    .strip_prefix(&dir)
                    .unwrap_or(f)
                    .to_string_lossy()
                    .to_string();
                let lc = rel.to_lowercase();
                (rel, lc)
            })
            .collect();
        self.status = format!("{} 個のmp3を取得: {}", self.files.len(), dir.display());
        self.root = Some(dir);
        self.selected = None;
        self.meta = Meta::default();
        self.orig = Meta::default();
        self.info.clear();
    }

    fn select(&mut self, idx: usize) {
        let Some(path) = self.files.get(idx).cloned() else {
            return;
        };
        match read_meta(&path) {
            Ok((m, info)) => {
                self.meta = m.clone();
                self.orig = m;
                self.info = info;
                self.selected = Some(idx);
            }
            Err(e) => {
                self.status = format!("読み込み失敗: {} ({})", path.display(), e);
            }
        }
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
        let Some(path) = self.selected.and_then(|i| self.files.get(i).cloned()) else {
            return;
        };
        match write_meta(&path, &self.meta) {
            Ok(_) => {
                self.orig = self.meta.clone();
                self.status = format!("保存しました: {}", path.display());
            }
            Err(e) => self.status = format!("保存失敗: {e}"),
        }
    }

    fn run_bulk(&mut self, p: Pending) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let targets: Vec<PathBuf> = if p.all {
            self.files.clone()
        } else {
            self.selected
                .and_then(|i| self.files.get(i).cloned())
                .into_iter()
                .collect()
        };
        let (mut ok, mut skipped, mut failed) = (0usize, 0usize, 0usize);
        for path in &targets {
            let (artist, album) = derive_from_dir(&root, path);
            let value = match p.kind {
                BulkKind::Album => album,
                BulkKind::Artist => artist,
            };
            let Some(v) = value else {
                skipped += 1;
                continue;
            };
            let kind = p.kind;
            let also = self.also_album_artist;
            let r = patch_tag(path, move |t| match kind {
                BulkKind::Album => t.set_album(v),
                BulkKind::Artist => {
                    t.set_artist(v.clone());
                    if also {
                        t.insert_text(ItemKey::AlbumArtist, v);
                    }
                }
            });
            match r {
                Ok(_) => ok += 1,
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
        self.status = match genres_json(&self.genres).and_then(|j| fs::write(&p, j).map_err(|e| e.to_string())) {
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
        let Some(path) = self.files.get(sel).cloned() else {
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

        // ---- ディレクトリ構成からの設定 (④) ----
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

        // ---- 左: ファイル一覧 ----
        let mut clicked: Option<usize> = None;
        egui::SidePanel::left("files")
            .resizable(true)
            .default_width(380.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("絞り込み");
                    ui.text_edit_singleline(&mut self.filter);
                });
                let q = self.filter.to_lowercase();
                let visible: Vec<usize> = (0..self.labels.len())
                    .filter(|&i| q.is_empty() || self.labels[i].1.contains(&q))
                    .collect();
                ui.weak(format!("{} / {} 件", visible.len(), self.labels.len()));
                ui.separator();
                let row_h = ui.text_style_height(&egui::TextStyle::Body) + 4.0;
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, row_h, visible.len(), |ui, range| {
                        for row in range {
                            let i = visible[row];
                            let sel = self.selected == Some(i);
                            if ui.selectable_label(sel, self.labels[i].0.clone()).clicked() {
                                clicked = Some(i);
                            }
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
                self.files.len()
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
            .with_inner_size([1100.0, 720.0])
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
