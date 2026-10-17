use crate::project::{Command, Photo, execute};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{fs, path::Path};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillManifest {pub version:u32,pub id:String,pub title:String,pub steps:Vec<Command>,#[serde(default)] pub export:bool}
#[derive(Clone, Debug)]
pub struct Skill {pub manifest:SkillManifest,pub instructions:String}
impl Skill {
    pub fn load(directory:&Path)->Result<Self>{
        let md=directory.join("SKILL.md");let json=directory.join("workflow.json");
        ensure!(fs::metadata(&md)?.len()<=100_000 && fs::metadata(&json)?.len()<=100_000,"Skill 文件过大。");
        let manifest:SkillManifest=serde_json::from_slice(&fs::read(json)?).context("Skill 清单无效：需要 workflow.json")?;
        ensure!(manifest.version==1 && !manifest.id.trim().is_empty() && manifest.steps.len()<=100,"不支持的 Skill 清单。");
        Ok(Self{manifest,instructions:fs::read_to_string(md)?})
    }
    pub fn preview(&self)->Vec<String>{self.manifest.steps.iter().map(|s|match s{
        Command::Portrait{smooth,tone}=>format!("磨皮 {:.0}% · 肤色统一 {:.0}%",smooth*100.0,tone*100.0),
        Command::Film=>"应用柔和胶片调色，曲线仍可编辑".into(),
        Command::Reset=>"重置修图参数".into(),
        Command::Rotate{clockwise}=>if *clockwise{"向右旋转 90°"}else{"向左旋转 90°"}.into(),
        Command::Crop{left,top,right,bottom,angle}=>format!("裁剪：左{:.0}% 上{:.0}% 右{:.0}% 下{:.0}%；旋转{angle:.1}°",left*100.0,top*100.0,right*100.0,bottom*100.0),
        Command::Mask{dab}=>format!("{}局部区域，画笔半径 {:.1}%",if dab.erase{"排除"}else{"添加"},dab.radius*100.0),
        Command::Heal{heal}=>format!("局部修复，画笔半径 {:.1}%",heal.radius*100.0),
        Command::Set{control,value}=>{
            let name=match control.as_str(){"light.exposure"=>"曝光","light.contrast"=>"对比度","light.highlights"=>"高光","light.shadows"=>"阴影","light.whites"=>"白色","light.blacks"=>"黑色","wb.temp"=>"色温","wb.tint"=>"色调","color.saturation"=>"饱和度","color.vibrance"=>"自然饱和度",_=>"自定义参数"};
            format!("{name}调整为 {value}")
        }
    }).chain(self.manifest.export.then_some("转到导出，选择目录后保存 sRGB JPEG".into())).collect()}
    pub fn apply(&self,photo:&mut Photo)->Result<()> {let mut next=photo.clone();for cmd in &self.manifest.steps {execute(&mut next,cmd)?;}*photo=next;Ok(())}
}
pub fn builtins()->Vec<Skill>{
    vec![
        Skill{manifest:SkillManifest{version:1,id:"natural-portrait".into(),title:"自然人像".into(),steps:vec![Command::Portrait{smooth:0.25,tone:0.15}],export:false},instructions:"先识别或手动指定人像区域，再应用适度磨皮与肤色统一。".into()},
        Skill{manifest:SkillManifest{version:1,id:"soft-film".into(),title:"基础胶片".into(),steps:vec![Command::Film],export:false},instructions:"柔和对比、轻微褪色；保留可编辑曲线。".into()},
        Skill{manifest:SkillManifest{version:1,id:"lumina-export".into(),title:"Lumina 展示导出".into(),steps:vec![],export:true},instructions:"导出 sRGB JPEG，默认移除 GPS，再由用户决定是否送入图集。".into()},
    ]
}
