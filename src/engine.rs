use anyhow::{Context, Result, ensure};
use lightcraft_develop::DevelopSettings;
use lightcraft_pipeline::{OutputDepth, RenderRequest, SourceInfo, StageCache, geometry::Frame, render_cached};
use lightcraft_raster::{Rgb32f,Rgba8};
use crate::project::{EditState, Photo, Retouch, SourceRecord, XmpReport, MAX_FILE, hash_file, validate_edit};
use std::{fs,io::Write,path::{Path,PathBuf},sync::{Arc,atomic::{AtomicBool,Ordering}},time::Instant};
use sha2::Digest;

pub struct Source {pub pixels:Arc<Rgb32f>,pub info:SourceInfo,pub metadata:lightcraft_meta::Metadata,pub width:usize,pub height:usize,pub hash:String,pub cache:StageCache,pub denoise_amount:f32}
impl Source {
    pub fn load(path:&Path,edge:usize)->Result<Self>{
        Self::load_with(path,edge,None)
    }
    pub fn load_photo(photo:&Photo,settings:&DevelopSettings,edge:usize)->Result<Self>{
        let amount=if photo.source.sensor{settings.denoise_amount()}else{0.0};
        let resource=if amount>0.0{let resource=photo.cached_denoise.as_ref().context("高级降噪结果尚未交接，请在完整版生成降噪后返回")?;resource.verify(&photo.source.sha256)?;Some((resource,amount))}else{None};
        let source=Self::load_with(&photo.source.path,edge,resource)?;
        ensure!(source.hash==photo.source.sha256,"原片已改变");Ok(source)
    }
    pub fn preview_key(photo:&Photo,settings:&DevelopSettings)->String{
        format!("{}:{:?}:{}",photo.source.sha256,photo.cached_denoise,settings.denoise_amount())
    }
    fn load_with(path:&Path,edge:usize,denoise:Option<(&crate::resources::CachedDenoise,f32)>)->Result<Self>{
        ensure!(fs::metadata(path)?.len()<=MAX_FILE,"原片超过 350MB。");
        let bytes=fs::read(path).context("无法读取原片")?;
        let hash=format!("{:x}",sha2::Sha256::digest(&bytes));
        let is_raw=lightcraft_raw::probe(&bytes).is_some();
        let metadata=lightcraft_meta::extract(&bytes);
        let dims=if is_raw {let info=lightcraft_raw::probe_info(&bytes).map_err(anyhow::Error::msg)?;info.developed_size()}else{let h=lightcraft_codecs::read_header(&bytes)?;(h.width as usize,h.height as usize)};
        ensure!(dims.0>0 && dims.1>0 && dims.0<=20000 && dims.1<=20000 && dims.0.saturating_mul(dims.1)<=100_000_000,"照片尺寸超出小样处理范围。");
        let (pixels,info,denoise_amount)=if let Some((resource,amount))=denoise {
            resource.validate(&hash)?;
            let spec=lightcraft_engine::media::DenoiseSpec{product:resource.path.clone(),key:resource.key.clone(),make:None};
            let (plain,twin,info)=lightcraft_engine::files::load_vec_pair(bytes,edge,&spec).map_err(anyhow::Error::msg)?;
            let twin=twin.context("高级降噪缓存不匹配此 RAW，不能用未降噪原片代替")?;
            ensure!(plain.width==twin.width && plain.height==twin.height,"降噪图像尺寸不符");
            let decoded=lightcraft_engine::media::DecodedSource{image:Arc::new(plain),info:Some(info.clone()),camera_tone:None,denoised:Some(Arc::new(lightcraft_engine::media::Twin::new(Arc::new(twin))))};
            (decoded.image_for(amount),info,amount)
        }else{let (pixels,info)=lightcraft_engine::files::load_vec(bytes,edge).map_err(anyhow::Error::msg)?;(Arc::new(pixels),info,0.0)};
        ensure!(!is_raw || info.raw,"这个 RAW 变体只能读取内嵌预览，当前 App 不会将它作为显影结果。");
        Ok(Self{pixels,info,metadata,width:dims.0,height:dims.1,hash,cache:StageCache::default(),denoise_amount})
    }
    pub fn default_settings(&self)->DevelopSettings {
        let mut settings=if self.info.raw {DevelopSettings::for_raw(self.info.as_shot_temp,self.info.as_shot_tint)}else{DevelopSettings::default()};
        if self.info.raw && !self.info.relative_wb {settings.wb.scale=lightcraft_develop::WbScale::Kelvin;}
        settings
    }
}
pub fn import_photo(path:&Path)->Result<(Photo,Source)>{
    let path=path.canonicalize()?;
    let source=Source::load(&path,1200)?;
    let mut settings=source.default_settings();let mut report=XmpReport::default();
    let sidecar=[path.with_extension("xmp"),path.with_extension("XMP")].into_iter().find(|p|p.is_file());
    let mut original_xmp=None;let mut xmp_sha256=None;
    if let Some(sidecar)=&sidecar {
        ensure!(fs::metadata(sidecar)?.len()<=2*1024*1024,"XMP 文件过大。");
        let text=fs::read_to_string(sidecar)?;
        (settings,report)=import_xmp(&text,&source)?;
        original_xmp=Some(text);xmp_sha256=Some(hash_file(sidecar)?);
    }
    let photo=Photo{source:SourceRecord{path,sha256:source.hash.clone(),xmp_path:sidecar,xmp_sha256,original_xmp,camera:source.metadata.model.clone().unwrap_or_default(),sensor:source.info.raw,width:source.width,height:source.height},baseline:source.default_settings(),edit:EditState{develop:settings,retouch:Retouch::default()},report,picked:true,catalog_ref:None,cached_denoise:None};
    validate_edit(&photo.edit)?;
    Ok((photo,source))
}
pub fn import_xmp(text:&str,source:&Source)->Result<(DevelopSettings,XmpReport)>{
    ensure!(text.len()<=2*1024*1024,"XMP 文件过大。");
    let parsed=lightcraft_meta::parse_xmp(text).map_err(anyhow::Error::msg)?;
    let names=["Temperature","Tint","WhiteBalance","Exposure2012","Contrast2012","Highlights2012","Shadows2012","Whites2012","Blacks2012","Saturation","Vibrance","ToneCurvePV2012","ToneCurvePV2012Red","ToneCurvePV2012Green","ToneCurvePV2012Blue","ToneCurve","ToneCurveRed","ToneCurveGreen","ToneCurveBlue","HasCrop","CropTop","CropLeft","CropBottom","CropRight","CropAngle"];
    let filtered=parsed.properties.iter().filter(|(key,_)|key.strip_prefix("crs:").is_some_and(|n|names.contains(&n))).map(|(k,v)|(k.clone(),v.clone())).collect();
    let target=if !source.info.raw {lightcraft_engine::crs::Target::Rendered}else if source.info.relative_wb {lightcraft_engine::crs::Target::RawRelative}else{lightcraft_engine::crs::Target::RawAbsolute};
    let (partial,skipped)=lightcraft_engine::crs::to_partial_report(&filtered,None,target,source.width as f64/source.height as f64);
    let settings=source.default_settings().merged(&partial)?;
    let mut report=XmpReport::default();
    for key in parsed.properties.keys().filter(|k|k.starts_with("crs:")) {
        let name=key.strip_prefix("crs:").unwrap_or(key);
        if !names.contains(&name)||skipped.iter().any(|s|s==name) {report.unsupported.push(name.into());}
        else if name.starts_with("Crop")||name=="HasCrop" {report.applied.push(name.into());}
        else {report.approximate.push(name.into());}
    }
    validate_edit(&EditState{develop:settings.clone(),retouch:Retouch::default()})?;
    Ok((settings,report))
}
pub struct RenderOutput {pub image:Rgba8,pub deep:lightcraft_pipeline::DeepImage,pub frame:Frame,pub elapsed_ms:u128}
/// Load a creative LUT from the photo's own library, never from another
/// process's registry or a same-named profile in a different library.
pub fn prepare_profile(photo:&Photo,edit:&EditState)->Result<bool>{
    let id=&edit.develop.profile.id;
    if !lightcraft_pipeline::lut::is_lut_profile(id){return Ok(false);}
    let path=profile_path(photo,edit)?;
    let lut=lightcraft_pipeline::lut::Lut3d::parse_cube(&fs::read_to_string(path)?).map_err(anyhow::Error::msg)?;
    let changed=lightcraft_pipeline::lut::get(id).is_none_or(|old|*old!=lut);
    if changed{lightcraft_pipeline::lut::register(id,lut);}
    Ok(changed)
}
fn profile_path(photo:&Photo,edit:&EditState)->Result<PathBuf>{
    let id=&edit.develop.profile.id;
    let reference=photo.catalog_ref.as_ref().context("LUT 缺少所属照片库，请重新关联工程照片")?;
    let prefs=reference.library.join("prefs.json");
    ensure!(fs::metadata(&prefs)?.len()<=16*1024*1024,"照片库配置过大");
    let value:serde_json::Value=serde_json::from_slice(&fs::read(prefs)?)?;
    let profiles:Vec<lightcraft_engine::cmd::lut_profiles::LutProfile>=serde_json::from_value(value.get("lutProfiles").cloned().unwrap_or_else(||serde_json::json!([])))?;
    let profile=profiles.iter().find(|p|&p.id==id).context("照片库中没有工程使用的 LUT，请恢复该配置文件")?;
    let path=Path::new(&profile.file);
    ensure!(path.is_absolute(),"LUT 必须使用绝对路径");
    ensure!(fs::metadata(path).with_context(||format!("LUT 文件缺失：{}",path.display()))?.len()<=64*1024*1024,"LUT 文件过大");
    Ok(path.to_path_buf())
}
pub fn profile_fingerprint(photo:&Photo)->Result<Option<String>>{
    if !lightcraft_pipeline::lut::is_lut_profile(&photo.edit.develop.profile.id){return Ok(None);}
    Ok(Some(hash_file(&profile_path(photo,&photo.edit)?)?))
}
pub fn render_photo(source:&Source,edit:&EditState,edge:usize,cancel:&AtomicBool)->Result<RenderOutput>{
    validate_edit(edit)?;
    ensure!(!source.info.raw || (edit.develop.denoise_amount()-source.denoise_amount).abs()<0.00001,"显影输入没有匹配当前降噪比例，请重新载入");
    ensure!(!lightcraft_pipeline::lut::is_lut_profile(&edit.develop.profile.id) || lightcraft_pipeline::lut::get(&edit.develop.profile.id).is_some(),"LUT 尚未加载，无法还原高级调色");
    ensure!(!cancel.load(Ordering::Relaxed),"操作已取消。");
    let started=Instant::now();
    let frame=lightcraft_pipeline::frame_for(&source.pixels,&source.info,&edit.develop,true);
    let (native_w,native_h)=frame.native_size();
    let mut req=RenderRequest::fit(edge.min(native_w.round().max(1.0)as usize),edge.min(native_h.round().max(1.0)as usize));req.depth=OutputDepth::U16;
    let mut result=render_cached(&source.pixels,&source.info,&edit.develop,&req,&source.cache);
    ensure!(!cancel.load(Ordering::Relaxed),"操作已取消。");
    let mut deep=result.deep.take().ok_or_else(||anyhow::anyhow!("显影引擎没有返回 16 位结果。"))?;
    crate::retouch::apply(&mut deep,&mut result.image,&edit.retouch,&frame,cancel)?;
    Ok(RenderOutput{image:result.image,deep,frame,elapsed_ms:started.elapsed().as_millis()})
}
pub fn export_photo(photo:&Photo,path:&Path,tiff:bool,keep_gps:bool,cancel:&AtomicBool)->Result<serde_json::Value>{
    export_photo_with_commit(photo,path,tiff,keep_gps,cancel,&||Ok(()))
}
pub fn export_photo_with_commit(photo:&Photo,path:&Path,tiff:bool,keep_gps:bool,cancel:&AtomicBool,commit:&impl Fn()->Result<()>)->Result<serde_json::Value>{
    let fingerprint=crate::project::photo_edit_hash(photo)?;
    photo.source.verify()?;
    ensure!(!path.exists(),"目标文件已存在，请使用新的文件名。");
    let parent=path.parent().unwrap_or(Path::new("."));fs::create_dir_all(parent)?;
    let started=Instant::now();let source=Source::load_photo(photo,&photo.edit.develop,20000)?;
    prepare_profile(photo,&photo.edit)?;
    ensure!(source.hash==photo.source.sha256,"原片已改变。");
    let out=render_photo(&source,&photo.edit,20000,cancel)?;
    let mut metadata=source.metadata;
    if let Some(text)=&photo.source.original_xmp{
        let parsed=lightcraft_meta::parse_xmp(text).map_err(anyhow::Error::msg)?;
        metadata.overlay_user_fields(&parsed.metadata);
        if parsed.values.contains_key("dc:subject"){metadata.keywords=parsed.metadata.keywords.clone();}
        if parsed.values.contains_key("lr:hierarchicalSubject"){metadata.hierarchical_keywords=parsed.metadata.hierarchical_keywords.clone();}
        metadata.fill_missing(&parsed.metadata);
        // These editable fields aren't covered by upstream overlay_user_fields.
        macro_rules! overlay {($($field:ident),*)=>{$(if parsed.metadata.$field.is_some(){metadata.$field=parsed.metadata.$field.clone();})*};}
        overlay!(city,state,country,sublocation,alt_text,extended_description);
    }
    if !keep_gps {metadata.gps=None;metadata.city=None;metadata.state=None;metadata.country=None;metadata.sublocation=None;}
    metadata.serial_number=None;metadata.lens_serial_number=None;metadata.regions.clear();metadata.orientation=Some(lightcraft_geom::Orientation::Normal);
    metadata.width=Some(out.image.width as u32);metadata.height=Some(out.image.height as u32);
    // Reuse LightCraft's export policy/encoding entry points after our retouch
    // and metadata filtering. Its session exporter would bypass these edits.
    use lightcraft_engine::export::{self,ExportFormat,ExportOptions};
    let options=ExportOptions{
        format:if tiff{ExportFormat::Tiff}else{ExportFormat::Jpeg},
        quality:94,bit_depth:Some(if tiff{16}else{8}),
        color_space:lightcraft_pipeline::OutputSpace::Srgb,
        ..Default::default()
    };
    let encoded=if tiff {
        ensure!(matches!(out.deep.samples,lightcraft_pipeline::DeepSamples::U16(_)),"需要 16 位样本。");
        export::encode_deep(&out.deep,&options,Some(&metadata))
    } else {export::encode_with_metadata(&out.image,&options,Some(&metadata))}.map_err(anyhow::Error::msg)?;
    ensure!(!cancel.load(Ordering::Relaxed),"操作已取消。");
    ensure!(crate::project::photo_edit_hash(photo)?==fingerprint,"导出期间 LUT 或元数据发生变化，请重试");
    commit()?;
    let mut file=fs::OpenOptions::new().create_new(true).write(true).open(path)?;
    if let Err(e)=file.write_all(&encoded).and_then(|_|file.sync_all()){drop(file);let _=fs::remove_file(path);return Err(e.into());}
    Ok(serde_json::json!({"path":path,"width":out.image.width,"height":out.image.height,"bits":if tiff{16}else{8},"icc":"sRGB","bytes":encoded.len(),"elapsed_ms":started.elapsed().as_millis(),"edit_sha256":fingerprint}))
}
pub fn unique_output(directory:&Path,stem:&str,tiff:bool)->PathBuf {
    let clean:String=stem.chars().filter(|c|*c!='/' && *c!='\\' && *c!=':' && !c.is_control()).take(80).collect();
    let ext=if tiff{"tif"}else{"jpg"};
    let stamp=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d|d.as_millis()).unwrap_or_default();
    directory.join(format!("{clean}-studio-{stamp}.{ext}"))
}
pub fn probe(path:&Path)->Result<serde_json::Value>{
    let started=Instant::now();let (photo,source)=import_photo(path)?;
    let out=render_photo(&source,&photo.edit,1200,&AtomicBool::new(false))?;
    Ok(serde_json::json!({"sensor_decoded":source.info.raw,"camera":photo.source.camera,"native_width":source.width,"native_height":source.height,"preview_width":out.image.width,"preview_height":out.image.height,"bits":16,"elapsed_ms":started.elapsed().as_millis(),"xmp":photo.report}))
}
