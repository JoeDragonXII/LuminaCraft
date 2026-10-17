use anyhow::{Context,Result,ensure};
use luminacraft::{bridge,worker::{self,Job,Outcome,Request}};
use std::{path::PathBuf,sync::{Arc,atomic::AtomicBool},time::Duration};
fn main()->Result<()> {
    let args:Vec<_>=std::env::args().skip(1).collect();
    let base=args.first().context("isolated API URL required")?;
    ensure!(base=="http://127.0.0.1:49865","audit only supports its isolated test port");
    let receipts=PathBuf::from(args.get(1).context("advanced receipt required")?);
    let report=PathBuf::from(args.get(2).context("new audit output required")?);
    ensure!(!report.exists(),"audit report already exists");
    let value=bridge::read::<serde_json::Value>(&receipts)?;
    let records=bridge::export_receipts(&value)?;
    let record=records.into_iter().find(|r|r.output.extension().is_some_and(|e|e.eq_ignore_ascii_case("jpg"))).context("no JPEG receipt")?;
    ensure!(record.origin.as_deref()==Some("advanced-export"),"not an advanced receipt");
    let source_hash=luminacraft::project::hash_file(&record.output)?;
    let (tx,rx)=worker::start(eframe::egui::Context::default());
    let cancel=Arc::new(AtomicBool::new(false));
    let send=|id,job|->Result<Outcome>{
        tx.send(Request{id,job,cancel:cancel.clone()})?;
        let response=rx.recv_timeout(Duration::from_secs(120))?;
        ensure!(response.id==id,"worker response mismatch");
        response.result.map_err(anyhow::Error::msg)
    };
    let password=std::env::var("LUMINACRAFT_AUDIT_PASSWORD")?;
    let before=match send(1,Job::Connect(base.clone(),password.clone()))?{Outcome::Connected(v)=>v.len(),_=>anyhow::bail!("unexpected connect result")};
    ensure!(before==0,"isolated library must start empty");
    let first=match send(2,Job::Publish(record,"Advanced receipt audit".into(),None,false))?{Outcome::Published(r)=>r,_=>anyhow::bail!("unexpected publish result")};
    ensure!(first.asset_id.is_some()&&first.collection_id.is_some(),"missing publish receipt");
    let again=match send(3,Job::Publish(first.clone(),"Advanced receipt audit".into(),None,false))?{Outcome::Published(r)=>r,_=>anyhow::bail!("unexpected retry result")};
    ensure!(again.asset_id==first.asset_id&&again.collection_id==first.collection_id,"retry changed receipt");
    // Repeating an upload after losing just its asset receipt must reuse content.
    let mut lost_asset=first.clone();lost_asset.asset_id=None;
    let uploaded_again=match send(5,Job::Publish(lost_asset,"Advanced receipt audit".into(),None,false))?{Outcome::Published(r)=>r,_=>anyhow::bail!("unexpected upload retry")};
    ensure!(uploaded_again.asset_id==first.asset_id,"content retry duplicated asset");
    let target=first.collection_id.clone();
    ensure!(send(6,Job::Publish(first.clone(),"".into(),target.clone(),false)).is_err(),"append accepted without confirmation");
    match send(7,Job::Publish(first.clone(),"".into(),target,true))?{Outcome::Published(_)=>{},_=>anyhow::bail!("unexpected append result")};
    let items=match send(4,Job::Connect(base.clone(),password))?{Outcome::Connected(v)=>v,_=>anyhow::bail!("unexpected listing")};
    ensure!(items.len()==1,"retry created duplicate drafts");
    ensure!(items[0]["count"].as_u64()==Some(1)&&items[0]["visibility"].as_str()==Some("draft"),"append duplicated membership or published draft");
    ensure!(luminacraft::project::hash_file(&first.output)?==source_hash,"upload modified output");
    bridge::write_new(&report,&serde_json::json!({"base":base,"receipt":first,"retryPreservedIds":true,"lostAssetReceiptDeduplicated":true,"confirmedAppendDeduplicated":true,"unconfirmedAppendRejected":true,"collections":items,"outputUnchanged":true}))?;
    println!("Real isolated API passed: advanced receipt upload, draft, worker retry, single collection, unchanged output.");
    Ok(())
}
