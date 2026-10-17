use anyhow::{Context, Result, bail, ensure};
use lightcraft_develop::DevelopSettings;
use lightcraft_geom::Point;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs, io::{Read, Write}, path::{Path, PathBuf}};

pub const SCHEMA: u32 = 1;
pub const MAX_PROJECT_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_FILE: u64 = 350 * 1024 * 1024;
pub fn edit_hash(edit:&EditState)->Result<String>{Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(edit)?)))}
pub fn photo_edit_hash(photo:&Photo)->Result<String>{
    let lut=crate::engine::profile_fingerprint(photo)?;
    if lut.is_some()||photo.source.original_xmp.is_some(){
        return Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(&serde_json::json!({"edit":photo.edit,"denoise":photo.cached_denoise,"lut":lut,"xmp":photo.source.original_xmp}))?)));
    }
    if photo.source.sensor && photo.edit.develop.denoise_amount()>0.0 {
        Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(&serde_json::json!({"edit":photo.edit,"denoise":photo.cached_denoise}))?)))
    }else{edit_hash(&photo.edit)}
}

pub fn hash_file(path: &Path) -> Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop { let n = file.read(&mut buf)?; if n == 0 {break;} hash.update(&buf[..n]); }
    Ok(format!("{:x}", hash.finalize()))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SourceRecord {
    pub path: PathBuf,
    pub sha256: String,
    pub xmp_path: Option<PathBuf>,
    pub xmp_sha256: Option<String>,
    pub original_xmp: Option<String>,
    pub camera: String,
    pub sensor: bool,
    pub width: usize,
    pub height: usize,
}
impl SourceRecord {
    pub fn verify_available(&self)->Result<()> {
        if self.path.exists(){ensure!(hash_file(&self.path)?==self.sha256,"原片已改变，不能套用旧编辑。");}
        if let (Some(path),Some(hash))=(&self.xmp_path,&self.xmp_sha256){if path.exists(){ensure!(hash_file(path)?==*hash,"原 XMP 已改变。");}}
        Ok(())
    }
    pub fn verify(&self) -> Result<()> {
        ensure!(hash_file(&self.path)? == self.sha256, "原片已改变，请重新导入；当前项目保持原样。");
        if let (Some(path), Some(hash)) = (&self.xmp_path, &self.xmp_sha256) {
            ensure!(hash_file(path)? == *hash, "原 XMP 已改变，请重新导入。");
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct XmpReport { pub applied: Vec<String>, pub approximate: Vec<String>, pub unsupported: Vec<String> }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FaceRegion {pub center: Point, pub radius: Point, pub excluded: Vec<Point>, #[serde(default)] pub polygon:Vec<Point>}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MaskDab {pub center: Point, pub radius: f64, pub erase: bool}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Heal {pub target: Point, pub source: Point, pub radius: f64}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Retouch { pub smooth: f64, pub tone: f64, pub faces: Vec<FaceRegion>, pub mask: Vec<MaskDab>, pub heals: Vec<Heal> }

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct EditState { pub develop: DevelopSettings, pub retouch: Retouch }
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Photo {
    pub source: SourceRecord, pub baseline: DevelopSettings, pub edit: EditState, pub report: XmpReport, pub picked: bool,
    #[serde(default,skip_serializing_if="Option::is_none")] pub catalog_ref:Option<CatalogRef>,
    #[serde(default,skip_serializing_if="Option::is_none")] pub cached_denoise:Option<crate::resources::CachedDenoise>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CatalogRef {pub library:PathBuf,pub id:u64,#[serde(default,skip_serializing_if="Option::is_none")] pub copy_name:Option<String>}
impl Photo {
    pub fn same_identity(&self,other:&Photo)->bool {
        match (&self.catalog_ref,&other.catalog_ref){
            (Some(a),Some(b))=>a.id==b.id && crate::bridge::same_path(&a.library,&b.library),
            (None,None)=>self.source.sha256==other.source.sha256,
            _=>false,
        }
    }
    pub fn thumbnail_key(&self)->String {match &self.catalog_ref{Some(r)=>format!("{}:{}:{}",r.library.display(),r.id,self.source.sha256),None=>self.source.sha256.clone()}}
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Version {pub photo: usize, pub label: String, pub edit: EditState}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishRecord {#[serde(default,skip_serializing_if="Option::is_none")] pub lumina_base:Option<String>,pub source_sha256: String, pub edit_sha256: String, pub output: PathBuf, pub output_sha256:String, pub asset_id: Option<String>, pub collection_id: Option<String>,#[serde(default,skip_serializing_if="Option::is_none")] pub origin:Option<String>}
impl PublishRecord {
    pub fn preview(&self)->Result<image::RgbaImage>{
        ensure!(fs::metadata(&self.output)?.len()<=MAX_FILE,"成片超过大小限制");
        let bytes=fs::read(&self.output)?;
        ensure!(format!("{:x}",Sha256::digest(&bytes))==self.output_sha256,"成片已改变，不能作为历史版本预览");
        let mut reader=image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
        let mut limits=image::Limits::default();limits.max_alloc=Some(600*1024*1024);limits.max_image_width=Some(20000);limits.max_image_height=Some(20000);reader.limits(limits);
        let image=reader.decode()?;
        Ok(if image.width()>1800||image.height()>1800{image.thumbnail(1800,1800)}else{image}.into_rgba8())
    }
    pub fn copy_with_commit(&self,path:&Path,commit:&impl Fn()->Result<()>)->Result<()>{
        ensure!(fs::metadata(&self.output)?.len()<=MAX_FILE,"成片超过大小限制");
        let bytes=fs::read(&self.output)?;
        ensure!(format!("{:x}",Sha256::digest(&bytes))==self.output_sha256,"成片已改变，不能复制");
        let format=|p:&Path|p.extension().map(|s|match s.to_string_lossy().to_ascii_lowercase().as_str(){"jpg"|"jpeg"=>"jpg".into(),"tif"|"tiff"=>"tif".into(),ext=>ext.to_owned()});
        ensure!(format(path)==format(&self.output),"副本需保留原成片格式");
        commit()?;
        let mut file=fs::OpenOptions::new().create_new(true).write(true).open(path)?;
        if let Err(error)=file.write_all(&bytes).and_then(|_|file.sync_all()){drop(file);let _=fs::remove_file(path);return Err(error.into());}
        Ok(())
    }
    pub fn bind_lumina(&mut self,base:&str)->Result<()>{
        let base=crate::lumina::normalize_base(base)?;
        ensure!(self.lumina_base.is_some()||(self.asset_id.is_none()&&self.collection_id.is_none()),"旧上传回执缺少站点归属，请重新导出成片后发送，避免复用其他网站的 ID");
        if self.lumina_base.as_deref()!=Some(&base){self.asset_id=None;self.collection_id=None;}
        self.lumina_base=Some(base);Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Project {pub schema: u32, pub id:String, pub title: String, pub engine_version: String, pub photos: Vec<Photo>, pub versions: Vec<Version>, pub publishes: Vec<PublishRecord>, #[serde(default,skip_serializing_if="Vec::is_empty")] pub advanced_outputs:Vec<PublishRecord>, #[serde(default,skip_serializing_if="Vec::is_empty")] pub style_outputs:Vec<crate::ai_style::StyleOutput>, #[serde(skip)] pub saved_digest:Option<String>}
impl Default for Project {
    fn default() -> Self {Self {schema: SCHEMA, id:format!("studio-{}-{}",std::process::id(),std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|t|t.as_nanos()).unwrap_or_default()), title:"新摄影项目".into(), engine_version: "lightcraft-b3d392b/photocraft-583425b".into(), photos:vec![], versions:vec![], publishes:vec![],advanced_outputs:vec![],style_outputs:vec![],saved_digest:None}}
}
impl Project {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == SCHEMA, "不支持的项目版本 {}", self.schema);
        ensure!(self.versions.len() <= 500 && self.publishes.len()<=1000 && self.advanced_outputs.len()<=1000 && self.title.len()<=512 && self.id.len()<=100, "工程版本、导出记录或名称超过容量。");
        for photo in &self.photos { if let Some(cached)=&photo.cached_denoise{cached.validate(&photo.source.sha256)?;} validate_edit(&photo.edit)?;validate_edit(&EditState{develop:photo.baseline.clone(),retouch:Retouch::default()})?; ensure!(photo.source.width.saturating_mul(photo.source.height) <= 100_000_000, "照片过大。"); }
        for version in &self.versions { ensure!(version.photo < self.photos.len(), "版本引用失效。"); validate_edit(&version.edit)?; }
        ensure!(self.style_outputs.len()<=1000,"风格版本超过 1000 个");
        let mut ids=std::collections::HashSet::new();
        for result in &self.style_outputs{result.validate()?;ensure!(result.project_id==self.id && ids.insert(&result.id),"风格版本的工程身份不符或 ID 重复");}
        Ok(())
    }
    pub fn digest(&self)->Result<String>{Ok(format!("{:x}",Sha256::digest(serde_json::to_vec(self)?)))}
    pub fn save(&mut self, directory: &Path) -> Result<()> {
        self.save_with_commit(directory,&||Ok(()))
    }
    pub fn save_with_commit(&mut self,directory:&Path,commit:&impl Fn()->Result<()>)->Result<()>{
        self.validate()?;
        for photo in &self.photos { photo.source.verify()?; }
        let bytes=serde_json::to_vec_pretty(self)?;
        ensure!(bytes.len()<MAX_PROJECT_BYTES,"工程数据超过 256 MiB，请减少版本记录后再保存");
        fs::create_dir_all(directory)?;
        let target = directory.join("project.json");
        if target.exists(){let old:Project=serde_json::from_slice(&fs::read(&target)?).context("已有项目文件无法读取，请另选目录保存。")?;ensure!(old.id==self.id && self.saved_digest.as_ref()==Some(&old.digest()?),"目录已有其他或更新的项目，请重新打开或另选目录，避免覆盖。");}
        ensure!(!self.photos.iter().any(|p| p.source.path == target || p.source.xmp_path.as_ref() == Some(&target)), "不能覆盖原文件。");
        let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|t|t.as_nanos()).unwrap_or_default();
        let tmp = directory.join(format!(".project-{}-{stamp}.tmp", std::process::id()));
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&tmp).context("无法创建项目临时文件，请检查目录权限。")?;
        let result = (|| {file.write_all(&bytes)?; file.sync_all()?; drop(file);commit()?; fs::rename(&tmp,&target)?; Ok(())})();
        if result.is_err() {let _ = fs::remove_file(&tmp);}
        if result.is_ok(){self.saved_digest=Some(self.digest()?);}
        result
    }
    pub fn open(directory: &Path) -> Result<Self> {
        Self::read_directory(directory,false)
    }
    pub fn open_catalog(directory:&Path)->Result<Self>{Self::read_directory(directory,true)}
    fn read_directory(directory:&Path,offline:bool)->Result<Self>{
        let path = directory.join("project.json");
        ensure!(fs::metadata(&path)?.len() < MAX_PROJECT_BYTES as u64, "项目文件过大。");
        let mut project: Self = serde_json::from_slice(&fs::read(path)?)?;
        project.validate()?;
        for photo in &project.photos {if offline{photo.source.verify_available()?;}else{photo.source.verify()?;}}
        project.saved_digest=Some(project.digest()?);
        for record in crate::ai_style::recover_outputs(directory,&project.id)? {
            if !project.style_outputs.iter().any(|r|r.id==record.id) && project.style_outputs.len()<1000{project.style_outputs.push(record);}
        }
        project.validate()?;Ok(project)
    }
    pub fn version(&mut self, photo: usize, label: &str) {
        if let Some(p) = self.photos.get(photo) { if self.versions.len() < 500 {self.versions.push(Version{photo,label:label.into(),edit:p.edit.clone()});} }
    }
}

fn point(p: Point) -> bool {p.x.is_finite() && p.y.is_finite() && (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y)}
pub fn validate_edit(edit: &EditState) -> Result<()> {
    let s = &edit.develop;
    for control in lightcraft_develop::CONTROLS {if let Some(value)=lightcraft_develop::controls::get(s,control.id){ensure!(value.is_finite() && value>=control.min && value<=control.max,"调色参数 {} 超出范围。",control.id);}}
    ensure!(s.masks.len()<=128 && s.spots.len()<=2000 && s.point_colors.len()<=64 && s.red_eye.len()<=128,"高级局部调整超过工程容量。");
    let native=serde_json::to_vec(s)?;
    ensure!(native.len()<=8*1024*1024,"高级显影参数过大。");
    let _:DevelopSettings=serde_json::from_slice(&native)?;
    ensure!((-5.0..=5.0).contains(&s.light.exposure) && (1000.0..=50000.0).contains(&s.wb.temp) && (-150.0..=150.0).contains(&s.wb.tint), "曝光或白平衡超出范围。");
    for x in [s.light.contrast,s.light.highlights,s.light.shadows,s.light.whites,s.light.blacks,s.color.saturation,s.color.vibrance] {ensure!((-100.0..=100.0).contains(&x), "调色参数超出范围。");}
    let c=s.crop.geometry;
    ensure!(c.angle.is_finite() && (-45.0..=45.0).contains(&c.angle) && c.rect.x0.is_finite() && c.rect.y0.is_finite() && c.rect.x1.is_finite() && c.rect.y1.is_finite() && c.rect.x0>=0.0 && c.rect.y0>=0.0 && c.rect.x1<=1.0 && c.rect.y1<=1.0 && c.rect.width()>=0.02 && c.rect.height()>=0.02, "裁剪范围无效。");
    for curve in [&s.curve.master,&s.curve.red,&s.curve.green,&s.curve.blue] {ensure!(curve.len()<=64 && curve.iter().all(|p| point(*p)), "曲线参数无效。");}
    let r=&edit.retouch;
    ensure!((0.0..=1.0).contains(&r.smooth) && (0.0..=1.0).contains(&r.tone), "修饰强度无效。");
    ensure!(r.faces.len()<=30 && r.mask.len()<=2000 && r.heals.len()<=200, "修饰操作过多。");
    for face in &r.faces {ensure!(point(face.center) && face.radius.x.is_finite() && face.radius.y.is_finite() && face.radius.x>0.0 && face.radius.x<=1.0 && face.radius.y>0.0 && face.radius.y<=1.0 && face.excluded.len()<=10 && face.excluded.iter().all(|p|point(*p)) && face.polygon.len()<=64 && face.polygon.iter().all(|p|point(*p)), "人像区域无效。");}
    for dab in &r.mask {ensure!(point(dab.center) && (0.001..=0.2).contains(&dab.radius), "蒙版画笔无效。");}
    for heal in &r.heals {ensure!(point(heal.target) && point(heal.source) && (0.001..=0.05).contains(&heal.radius), "修复画笔无效。");}
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag="tool",rename_all="snake_case",deny_unknown_fields)]
pub enum Command {
    Set {control:String,value:f64},
    Portrait {smooth:f64,tone:f64},
    Film,
    Reset,
    Rotate {clockwise:bool},
    Crop {left:f64,top:f64,right:f64,bottom:f64,angle:f64},
    Mask {dab:MaskDab},
    Heal {heal:Heal},
}
pub fn execute(photo: &mut Photo, command: &Command) -> Result<()> {
    let mut edit=photo.edit.clone();
    match command {
        Command::Set{control,value} => {
            let target = match control.as_str() {
                "light.exposure"=>&mut edit.develop.light.exposure,"light.contrast"=>&mut edit.develop.light.contrast,
                "light.highlights"=>&mut edit.develop.light.highlights,"light.shadows"=>&mut edit.develop.light.shadows,
                "light.whites"=>&mut edit.develop.light.whites,"light.blacks"=>&mut edit.develop.light.blacks,
                "wb.temp"=>&mut edit.develop.wb.temp,"wb.tint"=>&mut edit.develop.wb.tint,
                "color.saturation"=>&mut edit.develop.color.saturation,"color.vibrance"=>&mut edit.develop.color.vibrance,
                _=>bail!("不支持的编辑命令：{control}"),
            }; *target=*value;
            if control.starts_with("wb.") {edit.develop.wb.mode=lightcraft_develop::WbMode::Custom;}
        }
        Command::Portrait{smooth,tone} => {edit.retouch.smooth=*smooth;edit.retouch.tone=*tone;}
        Command::Film => {edit.develop.light.contrast=-8.0;edit.develop.color.saturation=-12.0;edit.develop.color.vibrance=10.0;edit.develop.curve.master=vec![Point::new(0.0,0.04),Point::new(0.25,0.23),Point::new(0.75,0.79),Point::new(1.0,0.97)];}
        Command::Reset=>{edit=EditState{develop:photo.baseline.clone(),retouch:Retouch::default()};}
        Command::Rotate{clockwise}=>{edit.develop.orientation=edit.develop.orientation.rotated(*clockwise);}
        Command::Crop{left,top,right,bottom,angle}=>{edit.develop.crop.geometry.rect=lightcraft_geom::Rect::new(*left,*top,*right,*bottom);edit.develop.crop.geometry.angle=*angle;}
        Command::Mask{dab}=>edit.retouch.mask.push(dab.clone()),
        Command::Heal{heal}=>edit.retouch.heals.push(heal.clone()),
    }
    validate_edit(&edit)?;
    photo.edit=edit;
    Ok(())
}

