use anyhow::{Result, ensure};
use lightcraft_geom::Point;
use lightcraft_pipeline::{DeepImage, DeepSamples, geometry::Frame};
use lightcraft_raster::{Rgb32f,Rgba8};
use crate::project::{FaceRegion, Retouch, hash_file};
use std::{path::Path,sync::atomic::{AtomicBool,Ordering}};

pub fn output_to_source(frame:&Frame,p:Point,w:usize,h:usize)->Point {
    let n=frame.out_to_norm(w,h).apply(Point::new(p.x*w as f64,p.y*h as f64));
    let n=frame.transformed_to_corrected(n);
    let (x,y)=frame.orient.inverse().map(n.x,n.y,1.0,1.0);
    Point::new(x,y)
}
pub fn source_to_output(frame:&Frame,p:Point,w:usize,h:usize)->Point {
    let (x,y)=frame.orient.map(p.x,p.y,1.0,1.0);
    frame.norm_to_out(w,h).apply(frame.corrected_to_transformed(Point::new(x,y)))
}

pub fn detect_faces(img:&Rgba8,frame:&Frame,model:&Path)->Result<Vec<FaceRegion>> {
    ensure!(hash_file(model)?==lightcraft_faces::known::YUNET_SHA256,"人脸模型校验失败。");
    let detector=lightcraft_faces::yunet::Detector::new(&std::fs::read(model)?).map_err(anyhow::Error::msg)?;
    let rgb:Vec<u8>=img.data.iter().flat_map(|p|p[..3].iter().copied()).collect();
    let faces=detector.detect(&rgb,img.width,img.height,&lightcraft_faces::yunet::Options{max_faces:30,..Default::default()}).map_err(anyhow::Error::msg)?;
    Ok(faces.into_iter().map(|f| {
        let cx=(f.x0+f.x1) as f64*0.5;let cy=f.y0 as f64+(f.y1-f.y0) as f64*0.54;
        let rx=(f.x1-f.x0) as f64*0.46;let ry=(f.y1-f.y0) as f64*0.51;
        let center=output_to_source(frame,Point::new(cx,cy),img.width,img.height);
        let polygon:Vec<Point>=(0..32).map(|n|{let t=n as f64*std::f64::consts::TAU/32.0;let p=output_to_source(frame,Point::new(cx+rx*t.cos(),cy+ry*t.sin()),img.width,img.height);Point::new(p.x.clamp(0.0,1.0),p.y.clamp(0.0,1.0))}).collect();
        let radius=Point::new(polygon.iter().map(|p|(p.x-center.x).abs()).fold(0.0,f64::max),polygon.iter().map(|p|(p.y-center.y).abs()).fold(0.0,f64::max));
        let excluded=f.landmarks.iter().map(|&(x,y)|{let p=output_to_source(frame,Point::new(x as f64,y as f64),img.width,img.height);Point::new(p.x.clamp(0.0,1.0),p.y.clamp(0.0,1.0))}).collect();
        FaceRegion{center:Point::new(center.x.clamp(0.0,1.0),center.y.clamp(0.0,1.0)),radius,excluded,polygon}
    }).collect())
}

fn polygon_contains(p:Point,vertices:&[Point])->bool {
    let mut inside=false;
    if vertices.len()<3{return false;}
    let mut prev=vertices.len()-1;
    for (n,v) in vertices.iter().enumerate(){let a=vertices[prev];if (v.y>p.y)!=(a.y>p.y) && p.x < (a.x-v.x)*(p.y-v.y)/(a.y-v.y)+v.x {inside=!inside;}prev=n;}
    inside
}
fn skin(p:[f32;3])->bool {
    let y=0.299*p[0]+0.587*p[1]+0.114*p[2];let cb=0.5+0.564*(p[2]-y);let cr=0.5+0.713*(p[0]-y);
    y>0.07 && y<0.98 && (0.25..0.57).contains(&cb) && (0.48..0.77).contains(&cr)
}
fn weight(r:&Retouch,p:Point,w:usize,h:usize)->f32 {
    let mut weight=0.0f64;
    for f in &r.faces {
        let inside=if f.polygon.is_empty(){((p.x-f.center.x)/f.radius.x).powi(2)+((p.y-f.center.y)/f.radius.y).powi(2)<1.0}else{polygon_contains(p,&f.polygon)};
        if inside {
            let protected=f.excluded.iter().any(|q|((p.x-q.x)/f.radius.x).powi(2)+((p.y-q.y)/f.radius.y).powi(2)<0.085);
            if !protected {weight=1.0;}
        }
    }
    let long=w.max(h) as f64;
    for d in &r.mask {
        let dist=(((p.x-d.center.x)*w as f64/long).powi(2)+((p.y-d.center.y)*h as f64/long).powi(2)).sqrt()/d.radius;
        if dist<1.0 {let k=((1.0-dist)*3.0).min(1.0);if d.erase{weight*=1.0-k;}else{weight=weight.max(k);}}
    }
    weight as f32
}

pub fn apply(deep:&mut DeepImage,preview:&mut Rgba8,r:&Retouch,frame:&Frame,cancel:&AtomicBool)->Result<()> {
    let DeepSamples::U16(samples)=&mut deep.samples else {return Ok(());};
    let (w,h)=(deep.width,deep.height);
    if r.smooth>0.0 || r.tone>0.0 {
        let data:Vec<[f32;3]>=samples.chunks_exact(3).map(|p|[p[0] as f32/65535.0,p[1] as f32/65535.0,p[2] as f32/65535.0]).collect();
        let image=Rgb32f{width:w,height:h,data};
        let blurred=lightcraft_raster::blur::gaussian(&image,(w.max(h) as f32*0.0025).max(0.8));
        for y in 0..h {
            if y%16==0 {ensure!(!cancel.load(Ordering::Relaxed),"操作已取消。");}
            for x in 0..w {
                let i=y*w+x;let p=image.data[i];let b=blurred.data[i];
                if !skin(p){continue;}
                let source=output_to_source(frame,Point::new((x as f64+0.5)/w as f64,(y as f64+0.5)/h as f64),w,h);
                let mask=weight(r,source,frame.src_w,frame.src_h);
                let delta=(0..3).map(|c|(p[c]-b[c]).abs()).fold(0.0,f32::max);
                let edge=(1.0-delta/0.12).clamp(0.0,1.0);
                let smooth=r.smooth as f32*0.65*mask*edge;
                let l=(p[0]+p[1]+p[2])/3.0;let bl=(b[0]+b[1]+b[2])/3.0;
                for c in 0..3 {let toned=(b[c]+l-bl).clamp(0.0,1.0);let v=p[c]+(b[c]-p[c])*smooth+(toned-p[c])*r.tone as f32*0.25*mask*edge;samples[i*3+c]=(v.clamp(0.0,1.0)*65535.0).round() as u16;}
            }
        }
    }
    for heal in &r.heals {
        ensure!(!cancel.load(Ordering::Relaxed),"操作已取消。");
        let target=source_to_output(frame,heal.target,w,h);let origin=source_to_output(frame,heal.source,w,h);
        if target.x<0.0 || target.y<0.0 || target.x>=w as f64 || target.y>=h as f64 {continue;}
        let radius=(heal.radius*frame.px_per_long(w)).max(1.0);
        ensure!(radius<=512.0,"修复画笔过大，请缩小后重试。");
        let x0=(target.x-radius-2.0).floor().max(0.0) as usize;let y0=(target.y-radius-2.0).floor().max(0.0) as usize;
        let x1=(target.x+radius+2.0).ceil().min(w as f64) as usize;let y1=(target.y+radius+2.0).ceil().min(h as f64) as usize;
        let (pw,ph)=(x1-x0,y1-y0);if pw<3 || ph<3{continue;}
        let mut dst=Vec::with_capacity(pw*ph*3);let mut src=Vec::with_capacity(pw*ph*3);let mut mask=Vec::with_capacity(pw*ph);
        for y in y0..y1 {for x in x0..x1 {
            let sx=((x as f64+origin.x-target.x).round() as i64).clamp(0,w as i64-1) as usize;
            let sy=((y as f64+origin.y-target.y).round() as i64).clamp(0,h as i64-1) as usize;
            for c in 0..3 {dst.push(samples[(y*w+x)*3+c] as f32/65535.0);src.push(samples[(sy*w+sx)*3+c] as f32/65535.0);}
            mask.push(x>x0 && x+1<x1 && y>y0 && y+1<y1 && (x as f64-target.x).hypot(y as f64-target.y)<radius);
        }}
        let healed=photocraft_algo::poisson::seamless_clone(pw,ph,3,&src,&dst,&mask);
        for y in 0..ph {for x in 0..pw {if mask[y*pw+x] {for c in 0..3{samples[((y0+y)*w+x0+x)*3+c]=(healed[(y*pw+x)*3+c].clamp(0.0,1.0)*65535.0).round() as u16;}}}}
    }
    for (p,rgb) in preview.data.iter_mut().zip(samples.chunks_exact(3)){for c in 0..3{p[c]=(rgb[c] as u32*255/65535) as u8;}}
    Ok(())
}
