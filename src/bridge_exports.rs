use super::*;
use crate::project::PublishRecord;
/// Completed files are standalone advanced outputs, never claimed to include main retouch.
pub fn export_receipts(value:&serde_json::Value)->Result<Vec<PublishRecord>> {
    let batches=value.get("batches").and_then(|v|v.as_array());
    let values:Vec<_>=match batches{Some(b)=>b.iter().collect(),None=>vec![value]};
    let mut records=Vec::new();
    for batch in values {
        let Some(files)=batch.get("files").and_then(|v|v.as_array())else{continue};
        for file in files {
            if file.get("error").is_some() || file.get("skipped").is_some(){continue;}
            let Some(path)=file.get("path").and_then(|v|v.as_str()).map(PathBuf::from)else{continue};
            ensure!(path.is_absolute(),"高级成片必须是绝对路径");
            let sha=if let Some(expected)=file.get("sha256").and_then(|v|v.as_str()){
                ensure!(expected.len()==64 && expected.bytes().all(|b|b.is_ascii_hexdigit()),"导出回执校验值无效");expected.to_string()
            }else{ensure!(path.is_file() && fs::metadata(&path)?.len()<=crate::project::MAX_FILE,"高级成片缺失或过大");hash_file(&path)?};
            // Keep the immutable receipt even if the file later disappears or changes.
            // Publishing verifies the actual bytes; a missing output must not block edit recovery.
            if records.iter().any(|r:&PublishRecord|r.output==path && r.output_sha256==sha){continue;}
            records.push(PublishRecord{lumina_base:None,source_sha256:String::new(),edit_sha256:String::new(),output:path,output_sha256:sha,asset_id:None,collection_id:None,origin:Some("advanced-export".into())});
        }
    }
    ensure!(records.len()<=1000,"高级导出回执过多");Ok(records)
}
pub fn export_batches(request:&Path)->Result<serde_json::Value> {
    let dir=request.with_file_name("Exports");let mut batches=Vec::new();
    if dir.exists(){let mut paths=fs::read_dir(dir)?.map(|e|e.map(|e|e.path())).collect::<std::io::Result<Vec<_>>>()?;paths.sort();for path in paths{if path.extension().is_some_and(|e|e=="json"){batches.push(read::<serde_json::Value>(&path)?);}}}
    Ok(serde_json::json!({"batches":batches}))
}
pub fn merge_export_receipts(project:&mut Project,value:&serde_json::Value)->Result<()> {
    let records=export_receipts(value)?;
    let mut next=project.advanced_outputs.clone();
    for record in records{if !next.iter().any(|r|r.output==record.output && r.output_sha256==record.output_sha256){next.push(record);}}
    ensure!(next.len()<=1000,"工程高级成片超过容量，现有回执完整保留");project.advanced_outputs=next;Ok(())
}
