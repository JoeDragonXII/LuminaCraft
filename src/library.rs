//! LuminaCraft's read-only-source adapter for the pinned LightCraft catalog.
use anyhow::{Context, Result, ensure};
use lightcraft_catalog::{Catalog, FsStore, Journal, LibraryLock, Op, Photo, PhotoId, Source};
use std::{collections::HashSet, fs, path::{Path, PathBuf}, sync::{Arc, atomic::{AtomicBool, Ordering}}};
#[path="library_folders.rs"] mod folders;
pub use folders::*;

pub struct Library {
    pub catalog: Catalog,
    pub directory: PathBuf,
    pub warning: Option<String>,
    journal: Journal,
    _lock: LibraryLock,
    paths: HashSet<PathBuf>,
}

pub fn default_directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("LUMINACRAFT_LIBRARY").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(path));
    }
    let base = std::env::var_os("LOCALAPPDATA").context("无法找到本机数据目录，请设置 LUMINACRAFT_LIBRARY。")?;
    Ok(PathBuf::from(base).join("LuminaCraft").join("Library"))
}

impl Library {
    pub fn save_file(&mut self)->Result<bool>{
        if crate::catalog_file::backing(&self.directory)?.is_none(){return Ok(false);}
        self.journal.snapshot(&self.catalog)?;
        crate::catalog_file::save(&self.directory)
    }
    pub fn open(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let lock = LibraryLock::acquire(directory, "LuminaCraft").map_err(|e| anyhow::anyhow!("照片库无法打开：{e}"))?;
        let (mut journal, mut catalog, report) = Journal::open(Box::new(FsStore::open(directory)?))?;
        crate::catalog_io::relocate(directory, &mut journal, &mut catalog)?;
        let warning = (report.failed > 0 || report.damaged.is_some() || report.torn_bytes > 0)
            .then(|| format!("照片库恢复报告：{} 项失败，{} 字节尾部恢复；损坏备份 {:?}", report.failed, report.torn_bytes, report.damaged));
        let paths = catalog.photos().filter_map(|p| photo_path(p).map(PathBuf::from)).collect();
        Ok(Self { catalog, directory: directory.to_path_buf(), warning, journal, _lock: lock, paths })
    }

    /// Validate the whole transaction before writing. Failed writes leave the view unchanged.
    pub fn apply(&mut self, ops: Vec<Op>) -> Result<()> {
        self.transact(ops).map(|_| ())
    }

    /// LightCraft inverse ops, durably committed as one transaction for undo/redo.
    pub fn transact(&mut self, ops: Vec<Op>) -> Result<Vec<Op>> {
        if ops.is_empty() { return Ok(Vec::new()); }
        let mut next = self.catalog.clone();
        let mut inverse=Vec::with_capacity(ops.len());
        for op in &ops { inverse.push(next.apply(op.clone())?); }
        self.journal.append(&ops)?;
        self.catalog = next;
        self.paths=self.catalog.photos().filter_map(|p|photo_path(p).map(PathBuf::from)).collect();
        if self.journal.wants_snapshot() {
            if let Err(e) = self.journal.snapshot(&self.catalog) {
                self.warning = Some(format!("索引整理失败，变更已保存在日志中：{e}"));
            }
        }
        inverse.reverse();
        Ok(inverse)
    }

    pub fn add(&mut self, photos: Vec<Photo>) -> Result<usize> {
        self.add_with_restore(photos,false).map(|(added,_)|added)
    }

    pub fn add_with_restore(&mut self, photos: Vec<Photo>,restore_removed:bool) -> Result<(usize,usize)> {
        let mut next = self.catalog.clone();
        let mut paths = self.paths.clone();
        let mut ops = Vec::new();
        let by_path:std::collections::HashMap<_,_>=self.catalog.photos().filter_map(|p|photo_path(p).map(|path|(PathBuf::from(path),(p.id,p.deleted)))).collect();
        let mut restored=HashSet::new();let mut added=0;
        for mut photo in photos {
            let Some(path) = photo_path(&photo).map(PathBuf::from) else { continue; };
            if !paths.insert(path.clone()) {
                if restore_removed{if let Some((id,true))=by_path.get(&path){if restored.insert(*id){ops.push(Op::SetDeleted{id:*id,deleted:false});}}}
                continue;
            }
            photo.id = next.alloc_photo_id();
            ops.push(Op::AddPhoto { photo: Box::new(photo) });
            added+=1;
        }
        self.apply(ops)?;
        self.paths = paths;
        Ok((added,restored.len()))
    }
}

pub fn photo_path(photo: &Photo) -> Option<&str> {
    match &photo.source { Source::File { path } => Some(path), _ => None }
}

pub fn supported(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s|
        ["arw", "jpg", "jpeg", "png", "tif", "tiff", "dng", "nef", "cr2", "cr3", "raf", "rw2", "orf", "pef"].contains(&s.to_ascii_lowercase().as_str()))
}

pub fn index_photo(path: &Path) -> Result<Photo> {
    let path = path.canonicalize()?;
    ensure!(fs::metadata(&path)?.len() <= crate::project::MAX_FILE, "照片超过 350MB。");
    let source = path.to_str().context("照片路径不是有效 Unicode。")?;
    let bytes = fs::read(&path)?;
    let info = lightcraft_engine::files::probe_bytes(source, &bytes).map_err(anyhow::Error::msg)?;
    ensure!(info.width > 0 && info.height > 0 && info.width <= 20000 && info.height <= 20000 && u64::from(info.width)*u64::from(info.height) <= 100_000_000, "照片尺寸过大。");
    let mut photo = Photo::new(PhotoId(0), Source::File { path: source.into() },
        path.file_name().and_then(|s| s.to_str()).unwrap_or("照片"), &info.format, info.width, info.height,
        &lightcraft_engine::import::system_clock());
    photo.kind = info.kind;
    photo.file_size = info.file_size;
    photo.captured = info.captured.or_else(||std::fs::metadata(&path).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d|lightcraft_engine::import::civil(d.as_secs() as i64)));
    photo.meta = info.meta;
    photo.content_hash = info.content_hash;
    photo.as_shot_wb = info.as_shot_wb;
    photo.preview_only = info.preview_only;
    Ok(photo)
}

pub enum ScanEvent { Photo(Box<Photo>), Error(String), Done { cancelled: bool } }

/// Incremental traversal. Symlinks/junctions and the catalog directory are skipped.
pub fn scan(paths: Vec<PathBuf>, skip: PathBuf, cancel: Arc<AtomicBool>, mut emit: impl FnMut(ScanEvent) -> bool) {
    let skip = skip.canonicalize().ok();
    let mut stack: Vec<_> = paths.into_iter().map(|p| (p, 0usize)).collect();
    let mut seen = HashSet::new();
    let mut count = 0usize;
    while let Some((path, depth)) = stack.pop() {
        if cancel.load(Ordering::Relaxed) { break; }
        let result = (|| -> Result<()> {
            let meta = fs::symlink_metadata(&path)?;
            if meta.file_type().is_symlink() || is_reparse(&meta) { return Ok(()); }
            let canonical = path.canonicalize()?;
            if skip.as_ref().is_some_and(|s| canonical.starts_with(s)) || !seen.insert(canonical) { return Ok(()); }
            if meta.is_dir() {
                ensure!(depth < 64, "文件夹嵌套超过 64 层。");
                for entry in fs::read_dir(&path)? {
                    if cancel.load(Ordering::Relaxed) { break; }
                    match entry {
                        Ok(entry) => { stack.push((entry.path(), depth+1)); ensure!(stack.len() <= 100_000, "待扫描路径过多，请分批添加文件夹。"); },
                        Err(e) => { if !emit(ScanEvent::Error(format!("{}：{e}", path.display()))) { cancel.store(true, Ordering::Relaxed); break; } }
                    }
                }
            } else if meta.is_file() && supported(&path) {
                count += 1;
                ensure!(count <= 100_000, "单次扫描超过 10 万张，请分批添加。");
                let photo = std::panic::catch_unwind(|| index_photo(&path))
                    .map_err(|_| anyhow::anyhow!("照片解析异常，已跳过此文件。"))??;
                if !emit(ScanEvent::Photo(Box::new(photo))) { cancel.store(true, Ordering::Relaxed); }
            }
            Ok(())
        })();
        if let Err(e) = result {
            if !emit(ScanEvent::Error(format!("{}：{e:#}", path.display()))) { break; }
            if count > 100_000 || stack.len() > 100_000 { break; }
        }
    }
    emit(ScanEvent::Done { cancelled: cancel.load(Ordering::Relaxed) });
}

#[cfg(windows)]
fn is_reparse(meta: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x400 != 0
}
#[cfg(not(windows))]
fn is_reparse(_: &fs::Metadata) -> bool { false }

/// Camera embedded previews are for browsing only. Editing still requires sensor decoding.
pub fn thumbnail(path: &str) -> Result<image::RgbaImage> {
    browsing_preview(path,320)
}

pub fn browsing_preview(path: &str, edge: usize) -> Result<image::RgbaImage> {
    let edge=edge.clamp(128,1600);
    ensure!(fs::metadata(path)?.len() <= crate::project::MAX_FILE, "照片过大。");
    if let Some(pixels) = lightcraft_engine::files::fs_preview_loader()(path, edge) {
        let rgba: Vec<u8> = pixels.data.iter().flatten().copied().collect();
        return image::RgbaImage::from_raw(pixels.width as u32, pixels.height as u32, rgba).context("缩略图数据无效。");
    }
    let source = crate::engine::Source::load(Path::new(path), edge)?;
    let edit = crate::project::EditState { develop: source.default_settings(), retouch: Default::default() };
    let pixels = crate::engine::render_photo(&source, &edit, edge, &AtomicBool::new(false))?.image;
    image::RgbaImage::from_raw(pixels.width as u32, pixels.height as u32, pixels.data.iter().flatten().copied().collect()).context("缩略图数据无效。")
}
