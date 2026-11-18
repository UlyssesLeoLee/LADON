//! Responsive UI state rules independent of the particular native renderer.
//! Sidebar collapse is REAL reflow, while hover-preview is a temporary overlay.
use serde::{Deserialize,Serialize};
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
pub enum Sidebar { Expanded, Icons, Hidden }
#[derive(Debug,Clone,Copy,PartialEq,Eq,Serialize,Deserialize)]
pub enum Chat { Collapsed, Expanded, Hidden }
#[derive(Debug,Clone,Copy,Serialize,Deserialize)]
pub struct LayoutInput { pub viewport_width:f32,pub viewport_height:f32,pub sidebar:Sidebar,pub chat:Chat,pub inspector:bool,pub hover_preview:bool }
#[derive(Debug,Clone,Copy,Serialize,Deserialize)]
pub struct Layout {pub sidebar_width:f32,pub content_width:f32,pub inspector_width:f32,pub chat_height:f32,pub hover_overlay_width:f32,pub status_visible:bool}
pub fn compute(input:LayoutInput)->Layout {
    let w=input.viewport_width.max(320.0);let h=input.viewport_height.max(280.0);
    let side=match input.sidebar{Sidebar::Expanded=>if w<800.0 {185.0} else {236.0},Sidebar::Icons=>60.0,Sidebar::Hidden=>0.0};
    let dock=match input.chat{Chat::Collapsed=>54.0,Chat::Expanded=>h.mul_add(0.46,0.0).clamp(220.0,h*0.65),Chat::Hidden=>0.0};
    let inspector=if input.inspector&&w-side>700.0 {(w*0.27).clamp(285.0,390.0)}else{0.0};
    Layout{sidebar_width:side,content_width:(w-side-inspector).max(0.0),inspector_width:inspector,
        chat_height:dock,hover_overlay_width:if input.hover_preview&&input.sidebar==Sidebar::Icons {206.0}else{0.0},
        status_visible:input.chat!=Chat::Hidden}
}
/// Non-critical activity status fades but warnings remain pinned.
pub fn status_opacity(idle_ms:u64,critical:bool,hovered:bool)->f32 {
    if critical || hovered || idle_ms<2300 {1.0}
    else if idle_ms>=4000 {0.0}
    else {1.0-(idle_ms-2300) as f32 / 1700.0}
}
#[cfg(test)]mod tests {use super::*;
 fn x(s:Sidebar)->LayoutInput{LayoutInput{viewport_width:1440.0,viewport_height:900.0,sidebar:s,chat:Chat::Collapsed,inspector:false,hover_preview:false}}
 #[test]fn ut_ui_reflow_fills_space(){let a=compute(x(Sidebar::Expanded));let b=compute(x(Sidebar::Icons));assert!((b.content_width-a.content_width-(a.sidebar_width-b.sidebar_width)).abs()<0.1);}
 #[test]fn ut_ui_hover_does_not_resize(){let mut a=x(Sidebar::Icons);a.hover_preview=true;assert_eq!(compute(a).content_width,compute(x(Sidebar::Icons)).content_width);assert!(compute(a).hover_overlay_width>0.0);}
 #[test]fn ut_ui_chat_bottom_height(){let mut a=x(Sidebar::Icons);a.chat=Chat::Expanded;assert!(compute(a).chat_height>200.0);}
 #[test]fn ut_ui_status_warning_stays(){assert_eq!(status_opacity(20000,true,false),1.0);assert_eq!(status_opacity(20000,false,false),0.0);}
}
