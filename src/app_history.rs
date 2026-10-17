//! Present the existing per-photo undo/redo stacks, without another undo engine.
//! Current/undone rows follow LightCraft panels/right.rs and PhotoCraft panels.rs.
use super::*;

fn move_to(past:&mut Vec<EditState>,now:&mut EditState,future:&mut Vec<EditState>,target:usize)->bool{
    if target>past.len()+future.len()||target==past.len(){return false;}
    while past.len()>target{let next=past.pop().unwrap();future.push(std::mem::replace(now,next));}
    while past.len()<target{let next=future.pop().unwrap();past.push(std::mem::replace(now,next));}
    true
}

fn change_name(before:&EditState,after:&EditState)->String{
    if before.retouch.heals!=after.retouch.heals{return "局部修复".into();}
    if before.retouch.mask!=after.retouch.mask{return "蒙版画笔".into();}
    if before.retouch.faces!=after.retouch.faces{return "人像区域".into();}
    if before.retouch!=after.retouch{return "人像修饰".into();}
    if before.develop.crop!=after.develop.crop{return "裁剪与旋转".into();}
    if before.develop.curve!=after.develop.curve{return "曲线".into();}
    let mut controls=lightcraft_develop::CONTROLS.iter().filter(|c|lightcraft_develop::controls::get(&before.develop,c.id)!=lightcraft_develop::controls::get(&after.develop,c.id));
    if let Some(c)=controls.next(){
        if controls.next().is_some(){return "多项调色调整".into();}
        let label=match c.id{"light.exposure"=>"曝光","light.contrast"=>"对比度","light.highlights"=>"高光","light.shadows"=>"阴影","light.whites"=>"白色","light.blacks"=>"黑色","wb.temp"=>"色温","wb.tint"=>"色调","color.saturation"=>"饱和度","color.vibrance"=>"自然饱和度",_=>c.label};
        if let Some(value)=lightcraft_develop::controls::get(&after.develop,c.id){return format!("{label}  {value:.2}");}
    }
    "调整参数".into()
}

impl StudioApp{
    fn history_states(&self)->(usize,Vec<&EditState>){
        let Some(photo)=self.project.photos.get(self.active)else{return (0,vec![])};
        let past=self.undo.get(&self.active).map(Vec::as_slice).unwrap_or_default();
        let future=self.redo.get(&self.active).map(Vec::as_slice).unwrap_or_default();
        (past.len(),past.iter().chain(std::iter::once(&photo.edit)).chain(future.iter().rev()).collect())
    }
    pub(super) fn jump_history(&mut self,target:usize){
        if self.blocking{return;}
        let Some(photo)=self.project.photos.get_mut(self.active)else{return;};
        if !move_to(self.undo.entry(self.active).or_default(),&mut photo.edit,self.redo.entry(self.active).or_default(),target){return;}
        let c=photo.edit.develop.crop.geometry;self.crop=[c.rect.x0,c.rect.y0,c.rect.x1,c.rect.y1,c.angle];
        self.dirty=true;self.before=false;self.grouping=None;self.heal_source=None;self.pending_preview=None;
        self.ai.reset_view();self.render();
    }
    pub(super) fn history_panel(&mut self,ui:&mut egui::Ui){
        lightcraft_ui_egui::widgets::register(ui.ctx(),"luminacraft.history.panel",ui.max_rect());
        let mut target=None;
        crate::chrome::centered_row(ui,30.0,|ui|{
            ui.label(RichText::new("操作历史").font(crate::typography::semibold(13.0))).on_hover_text("点击步骤回退或重做；回退后继续调整会替换后续步骤。会话历史不随工程保存，可用工程版本保留跨次状态。");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center),|ui|{ui.weak("本次会话");});
        });
        {
            let(cursor,states)=self.history_states();
            if states.is_empty(){ui.weak("选择照片后开始记录");return;}
            ui.spacing_mut().item_spacing.y=0.0;
            egui::ScrollArea::vertical().id_salt(("edit-history",self.active)).auto_shrink([false,false]).show_rows(ui,34.0,states.len(),|ui,range|{
                for row in range{
                    let i=states.len()-1-row;
                    let name=if i==0{"起始状态".into()}else{change_name(states[i-1],states[i])};
                    let text=RichText::new(&name).color(if i>cursor{crate::chrome::MUTED}else{crate::chrome::INK});
                    let r=ui.add_enabled(!self.blocking,egui::Button::selectable(i==cursor,text).min_size(Vec2::new(ui.available_width(),34.0)).truncate()).on_hover_text(if i>cursor{format!("重做到：{name}")}else{format!("回到：{name}")});
                    lightcraft_ui_egui::widgets::register(ui.ctx(),format!("luminacraft.history.step.{i}"),r.rect);
                    if r.clicked(){target=Some(i);}
                }
            });
        }
        if let Some(target)=target{self.jump_history(target);}
    }
    pub(super) fn history_diagnostic(&self)->serde_json::Value{
        let(cursor,states)=self.history_states();
        let labels:Vec<_>=states.iter().enumerate().map(|(i,s)|if i==0{"起始状态".into()}else{change_name(states[i-1],s)}).collect();
        serde_json::json!({"photo":self.active,"cursor":cursor,"labels":labels,"sessionOnly":true})
    }
}

#[cfg(test)]mod tests{
    use super::*;
    fn state(value:f64)->EditState{let mut s=EditState{develop:Default::default(),retouch:Default::default()};s.develop.light.exposure=value;s.retouch.smooth=value/10.0;s}
    #[test]fn passive_parameter_rows_do_not_round_or_create_edits(){
        let ctx=egui::Context::default();
        for(original,range,step,decimals)in [(0.8,-5.0..=5.0,0.05,2),(6611.234,2000.0..=50000.0,1.0,0),(0.37,0.0..=1.0,0.01,2)]{
            let mut value=original;
            for _ in 0..3{let mut changed=false;let output=ctx.run_ui(Default::default(),|ui|{changed=StudioApp::parameter_row(ui,"fixture",&mut value,range.clone(),step,decimals).1;});output.drop_without_applying_deltas();assert!(!changed);assert_eq!(value,original);}
        }
    }
    #[test]fn timeline_jumps_preserve_both_directions_and_invalid_targets(){
        let(mut past,mut now,mut future)=(vec![state(0.0),state(1.0)],state(2.0),vec![]);
        assert!(move_to(&mut past,&mut now,&mut future,0));assert_eq!(now,state(0.0));assert_eq!(future.len(),2);
        assert!(move_to(&mut past,&mut now,&mut future,2));assert_eq!(now,state(2.0));assert!(future.is_empty());
        assert!(!move_to(&mut past,&mut now,&mut future,3));assert_eq!(now,state(2.0));
        assert!(move_to(&mut past,&mut now,&mut future,1));assert_eq!(now,state(1.0));
        // A new edit keeps the past and discards the former future, as remember() does.
        past.push(now.clone());future.clear();now=state(3.0);
        assert!(move_to(&mut past,&mut now,&mut future,1));assert_eq!(now,state(1.0));
        assert!(move_to(&mut past,&mut now,&mut future,2));assert_eq!(now,state(3.0));
    }
}
