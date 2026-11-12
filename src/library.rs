use crate::library_catalogue::{Category, Collection, MISSING_ROWS};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use opencdi::frontend::{ART_SIZE_MAX, ART_SIZE_MIN};
use opencdi::frontend::DisplayMode;

pub const W: usize = 768;
pub const H: usize = 560;
const MAX_ITEMS: usize = 512;
pub const TOOLBAR_HEIGHT: usize = 42;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolbarPopup { Settings, About }

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ToolbarAction { OpenDisc, PlayDisc(char), Controller, Start220, Start490, Settings, About, Exit, AddDiscFolder, Bios220, Bios490, Borderless, Decorated, Windowed }

const SETTINGS_X: usize = 539;
const SETTINGS_Y: usize = TOOLBAR_HEIGHT + 4;
const SETTINGS_WIDTH: usize = 215;
const SETTINGS_ROW: usize = 22;
const SETTINGS_HEADER: usize = 0;
const SETTINGS_ITEMS: [(&str, ToolbarAction); 6] = [
    ("Borderless Fullscreen", ToolbarAction::Borderless),
    ("Decorated Fullscreen", ToolbarAction::Decorated),
    ("Windowed", ToolbarAction::Windowed),
    ("Add Disc Folder…", ToolbarAction::AddDiscFolder),
    ("BIOS 220", ToolbarAction::Bios220),
    ("BIOS 490", ToolbarAction::Bios490),
];

#[derive(Clone, Copy)]
pub struct LibraryLayout {
    pub columns: usize,
    pub rows: usize,
    pub cell_w: usize,
    pub cell_h: usize,
    pub gap: usize,
    pub left: usize,
    pub top: usize,
}

impl LibraryLayout {
    pub fn visible_count(self) -> usize { self.columns * self.rows }
}

pub fn layout(art_size: u8) -> LibraryLayout {
    let size = art_size.clamp(ART_SIZE_MIN, ART_SIZE_MAX);
    let range = ART_SIZE_MAX - ART_SIZE_MIN;
    let (columns, rows) = if size <= ART_SIZE_MIN + range / 3 {
        (10, 4)
    } else if size <= ART_SIZE_MIN + 2 * range / 3 {
        (8, 3)
    } else {
        (6, 2)
    };
    let gap = 10;
    let left = 14;
    let top = 108;
    let bottom = H - 105;
    let cell_w = (W - 2 * left - gap * (columns - 1)) / columns;
    let cell_h = (bottom - top - gap * (rows - 1)) / rows;
    LibraryLayout { columns, rows, cell_w, cell_h, gap, left, top }
}

#[derive(Clone)]
pub struct Artwork { pub w: usize, pub h: usize, pub pixels: Arc<[u32]> }

/// Draws menu geometry on the window-sized surface while keeping layout coordinates
/// in the emulator UI's reference space. This avoids enlarging a low-resolution menu.
pub struct NativeCanvas<'a> {
    dst: &'a mut [u32], pub width: usize, pub height: usize,
    view: (usize, usize, usize, usize),
    labels: Vec<NativeLabel>,
}

struct NativeLabel { x: usize, y: usize, value: String, color: u32, scale: usize, bounds: Option<(usize, usize)>, centered: bool }

impl<'a> NativeCanvas<'a> {
    pub(crate) fn surface(&mut self, x:usize,y:usize,w:usize,h:usize,r:usize,top:u32,bottom:u32,border:u32) {
        let (left,upper,right,lower) = (self.x(x),self.y(y),self.x(x+w).min(self.width),self.y(y+h).min(self.height));
        if right <= left || lower <= upper { return; }
        let (hw,hh)=((right-left) as f32/2.0,(lower-upper) as f32/2.0);
        let radius=(r as f32*self.view.2 as f32/W as f32).min(hw).min(hh);
        for py in upper..lower {
            let t=(py-upper) as f32/(lower-upper) as f32;
            let mut color=0xFF000000;
            for shift in [16,8,0] {
                let a=((top>>shift)&255) as f32; let b=((bottom>>shift)&255) as f32;
                let texture=if py%3==0 {1.0} else {0.0};
                color|=((a+(b-a)*t+texture).clamp(0.0,255.0) as u32)<<shift;
            }
            for px in left..right {
                let qx=((px-left) as f32+0.5-hw).abs()-(hw-radius);
                let qy=((py-upper) as f32+0.5-hh).abs()-(hh-radius);
                let outer=if qx>0.0 && qy>0.0 {qx.hypot(qy)}else{qx.max(qy).max(0.0)};
                let distance=outer+qx.max(qy).min(0.0)-radius;
                let alpha=((0.5-distance).clamp(0.0,1.0)*255.0) as u32;
                if alpha==0 {continue;}
                let paint=if distance > -1.0 {border}else{color};
                let at=py*self.width+px;
                self.dst[at]=if alpha==255 {paint}else{blend(paint,self.dst[at],alpha)};
            }
        }
    }

    pub(crate) fn icon(&mut self, kind:u8,x:usize,y:usize,size:usize,color:u32) {
        let mut lines:Vec<(f32,f32,f32,f32)>=Vec::new();
        let mut circles:Vec<(f32,f32,f32)>=Vec::new();
        match kind {
            0=>lines.extend([(2.,6.,2.,17.),(2.,17.,17.,17.),(17.,17.,19.,8.),(19.,8.,5.,8.),(5.,8.,2.,17.),(2.,6.,2.,4.),(2.,4.,8.,4.),(8.,4.,10.,6.),(10.,6.,17.,6.)]),
            1=>{lines.extend([(5.,5.,15.,5.),(15.,5.,17.,7.),(17.,7.,19.,15.),(19.,15.,16.,16.),(16.,16.,13.,12.),(13.,12.,7.,12.),(7.,12.,4.,16.),(4.,16.,1.,15.),(1.,15.,3.,7.),(3.,7.,5.,5.),(5.,7.,5.,11.),(3.,9.,7.,9.)]);circles.extend([(13.,8.,0.7),(16.,10.,0.7)]);}
            2=>{circles.extend([(10.,10.,6.),(10.,10.,2.3)]);for i in 0..8 {let a=i as f32*std::f32::consts::TAU/8.;let(s,c)=a.sin_cos();lines.push((10.+c*6.,10.+s*6.,10.+c*9.,10.+s*9.));}}
            3=>{circles.extend([(10.,10.,8.),(10.,6.,0.6)]);lines.extend([(10.,9.,10.,14.)]);}
            4=>{circles.extend([(10.,10.,8.),(10.,10.,1.7)]);lines.extend([(12.,4.,16.,8.),(4.,12.,8.,16.)]);}
            5=>{circles.push((10.,11.,7.));lines.push((10.,2.,10.,10.));}
            6=>{circles.push((8.,8.,5.));lines.push((12.,12.,18.,18.));}
            7|8=>{for (a,b) in [(3.,3.),(12.,3.),(3.,12.),(12.,12.)]{lines.extend([(a,b,a+5.,b),(a+5.,b,a+5.,b+5.),(a+5.,b+5.,a,b+5.),(a,b+5.,a,b)]);}}
            9=>{lines.extend([(3.,3.,17.,3.),(17.,3.,17.,17.),(17.,17.,3.,17.),(3.,17.,3.,3.),(4.,14.,8.,10.),(8.,10.,12.,14.),(12.,14.,15.,11.),(15.,11.,17.,13.)]);circles.push((12.,7.,1.4));}
            10=>{lines.extend([(8.,15.,8.,4.),(8.,4.,17.,2.),(17.,2.,17.,13.),(8.,7.,17.,5.)]);circles.extend([(5.,15.,2.5),(14.,14.,2.5)]);}
            11=>lines.extend([(2.,4.,18.,4.),(18.,4.,18.,17.),(18.,17.,2.,17.),(2.,17.,2.,4.),(2.,8.,18.,8.),(7.,4.,5.,8.),(13.,4.,11.,8.),(8.,10.,8.,15.),(8.,15.,13.,12.5),(13.,12.5,8.,10.)]),
            12=>lines.extend([(6.,5.,1.,10.),(1.,10.,6.,15.),(14.,5.,19.,10.),(19.,10.,14.,15.),(12.,3.,8.,17.)]),
            14=>lines.extend([(3.,2.,14.,2.),(14.,2.,18.,6.),(18.,6.,18.,18.),(18.,18.,3.,18.),(3.,18.,3.,2.),(6.,2.,6.,8.),(6.,8.,14.,8.),(14.,8.,14.,2.),(6.,18.,6.,12.),(6.,12.,15.,12.),(15.,12.,15.,18.)]),
            15=>lines.extend([(3.,8.,3.,18.),(3.,18.,17.,18.),(17.,18.,17.,8.),(10.,14.,10.,2.),(10.,2.,5.,7.),(10.,2.,15.,7.)]),
            16=>lines.extend([(10.,3.,3.,10.),(3.,10.,10.,17.),(3.,10.,18.,10.)]),
            17=>lines.extend([(5.,3.,17.,10.),(17.,10.,5.,17.),(5.,17.,5.,3.)]),
            18=>{circles.extend([(10.,10.,6.),(10.,10.,1.5)]);lines.extend([(10.,0.,10.,5.),(10.,15.,10.,20.),(0.,10.,5.,10.),(15.,10.,20.,10.)]);}
            19=>lines.extend([(6.,2.,14.,2.),(14.,2.,17.,5.),(17.,5.,17.,15.),(17.,15.,14.,18.),(14.,18.,6.,18.),(6.,18.,3.,15.),(3.,15.,3.,5.),(3.,5.,6.,2.),(10.,4.,10.,8.),(3.,10.,17.,10.)]),
            _=>{circles.push((10.,6.,3.));lines.extend([(3.,18.,10.,10.),(10.,10.,17.,18.),(17.,18.,3.,18.)]);}
        }
        let (left,upper,right,lower)=(self.x(x),self.y(y),self.x(x+size).min(self.width),self.y(y+size).min(self.height));
        let scale=(right-left).max(1) as f32/20.;
        for py in upper..lower {for px in left..right {
            let u=(px-left) as f32/scale;let v=(py-upper) as f32/((lower-upper).max(1) as f32/20.);
            let mut distance=f32::INFINITY;
            for &(ax,ay,bx,by) in &lines {let dx=bx-ax;let dy=by-ay;let t=(((u-ax)*dx+(v-ay)*dy)/(dx*dx+dy*dy)).clamp(0.,1.);distance=distance.min((u-ax-t*dx).hypot(v-ay-t*dy));}
            for &(cx,cy,r) in &circles {if kind==5 && v<6. {continue;}distance=distance.min(((u-cx).hypot(v-cy)-r).abs());}
            let alpha=((0.8-distance)*scale+0.5).clamp(0.,1.);
            if alpha>0. {let at=py*self.width+px;self.dst[at]=blend(color,self.dst[at],(alpha*255.) as u32);}
        }}
    }
    pub fn new(dst: &'a mut [u32], width: usize, height: usize, view: (usize, usize, usize, usize)) -> Self {
        let len=width.saturating_mul(height).min(dst.len());
        dst[..len].fill(0xFF080E17);
        Self { dst, width, height, view, labels: Vec::new() }
    }
    fn overlay(dst:&'a mut [u32],width:usize,height:usize,view:(usize,usize,usize,usize))->Self{Self{dst,width,height,view,labels:Vec::new()}}
    fn x(&self, x: usize) -> usize { self.view.0 + x * self.view.2 / W }
    fn y(&self, y: usize) -> usize { self.view.1 + y * self.view.3 / H }
    pub fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, color: u32) {
        let x0=self.x(x).min(self.width); let y0=self.y(y).min(self.height);
        let x1=self.x(x.saturating_add(w)).min(self.width); let y1=self.y(y.saturating_add(h)).min(self.height);
        for py in y0..y1 { let a=py*self.width+x0; let b=py*self.width+x1; if a<b && b<=self.dst.len() { self.dst[a..b].fill(color); } }
    }
    pub fn text(&mut self, x: usize, y: usize, value: &str, color: u32, scale: usize) {
        self.labels.push(NativeLabel { x, y, value: value.chars().take(96).collect(), color, scale, bounds: None, centered: false });
    }
    pub(crate) fn centered_text(&mut self, x: usize, y: usize, value: &str, color: u32, scale: usize) {
        self.text(x, y, value, color, scale);
        if let Some(label) = self.labels.last_mut() { label.centered = true; }
    }
    pub fn round_rect(&mut self,x:usize,y:usize,w:usize,h:usize,r:usize,color:u32){
        let r=r.min(w/2).min(h/2); if r==0 {self.rect(x,y,w,h,color);return;}
        self.rect(x+r,y,w-2*r,h,color); self.rect(x,y+r,w,h-2*r,color);
        for dy in 0..r { for dx in 0..r {
            let px=(r-dx) as i64; let py=(r-dy) as i64;
            if px*px+py*py <= (r as i64)*(r as i64) {
                self.rect(x+dx,y+dy,1,1,color); self.rect(x+w-1-dx,y+dy,1,1,color);
                self.rect(x+dx,y+h-1-dy,1,1,color); self.rect(x+w-1-dx,y+h-1-dy,1,1,color);
            }
        }}
    }
    pub fn line(&mut self,x:usize,y:usize,w:usize,color:u32){self.rect(x,y,w,1,color);}
    #[allow(dead_code)]
    fn legacy_text(&mut self, x: usize, y: usize, value: &str, color: u32, scale: usize) {
        let mut px=x;
        for ch in value.chars().take(64) {
            if ch==' ' { px+=4*scale; continue; }
            let rows=glyph(ch);
            for (gy,row) in rows.iter().enumerate() { for gx in 0..5 {
                if row & (1 << (4-gx)) != 0 { self.rect(px+gx*scale,y+gy*scale,scale,scale,color); }
            }}
            px+=6*scale;
        }
    }
    pub fn art_fit(&mut self, art: &Artwork, x: usize, y: usize, w: usize, h: usize) {
        let px=self.x(x); let py=self.y(y);
        let pw=self.x(x.saturating_add(w)).saturating_sub(px); let ph=self.y(y.saturating_add(h)).saturating_sub(py);
        blit_fit_scaled(self.dst,self.width,self.height,art,px,py,pw,ph);
    }
    pub fn cover_fit(&mut self, art: &Artwork, x: usize, y: usize, w: usize, h: usize) {
        let px = self.x(x);
        let py = self.y(y);
        let pw = self.x(x.saturating_add(w)).saturating_sub(px);
        let ph = self.y(y.saturating_add(h)).saturating_sub(py);
        blit_cover_fit(self.dst, self.width, self.height, art, px, py, pw, ph);
    }

    pub fn ring_art(&mut self,art:&Artwork,box_xywh:(usize,usize,usize,usize),center:(f32,f32),radius:f32,color:u32){
        if art.w==0||art.h==0{return;}
        let(x,y,w,h)=box_xywh;let px=self.x(x);let py=self.y(y);let pw=self.x(x+w).saturating_sub(px);let ph=self.y(y+h).saturating_sub(py);
        if pw==0||ph==0{return;}
        let scale=(pw as f32/art.w as f32).min(ph as f32/art.h as f32);
        let ox=px as f32+(pw as f32-art.w as f32*scale)/2.0;let oy=py as f32+(ph as f32-art.h as f32*scale)/2.0;
        let sx=self.view.2 as f32/W as f32;let sy=self.view.3 as f32/H as f32;let unit=sx.min(sy);
        let cx=((ox+center.0*scale-self.view.0 as f32)/sx).round().max(0.0) as usize;
        let cy=((oy+center.1*scale-self.view.1 as f32)/sy).round().max(0.0) as usize;
        let r=(radius*scale/unit).round().max(1.0) as usize;self.ring(cx,cy,r,color);
    }
    pub fn ring(&mut self, cx: usize, cy: usize, radius: usize, color: u32) {
        let scale=(self.view.2 as f32/W as f32).min(self.view.3 as f32/H as f32);
        let rx=(radius as f32*scale).round().max(1.0) as isize; let ry=rx;
        let thick=(2.0*scale).round().max(1.0) as isize;
        let inner=((rx-thick).max(0) as f32/rx as f32).powi(2);
        let px=self.x(cx) as isize; let py=self.y(cy) as isize;
        for dy in -ry..=ry { for dx in -rx..=rx {
            let q=(dx as f32/rx as f32).powi(2)+(dy as f32/ry as f32).powi(2);
            if inner<=q&&q<=1.0 { let xx=px+dx; let yy=py+dy; if xx>=0&&yy>=0&&xx<self.width as isize&&yy<self.height as isize { self.dst[yy as usize*self.width+xx as usize]=color; } }
        }}
    }
}

impl Drop for NativeCanvas<'_> {
    fn drop(&mut self) { self.flush_labels(); }
}

impl NativeCanvas<'_> {
    #[cfg(windows)]
    fn flush_labels(&mut self) {
        draw_labels_gdi(self);
        self.labels.clear();
    }

    #[cfg(not(windows))]
    fn flush_labels(&mut self) {
        let labels=std::mem::take(&mut self.labels);
        for label in labels {
            if let Some((right, bottom)) = label.bounds {
                let max_chars = (right.saturating_sub(self.x(label.x)) / (self.view.2 * 6 / W).max(1)).max(1);
                let mut y = label.y;
                let mut line = String::new();
                for word in label.value.split_whitespace() {
                    if !line.is_empty() && line.chars().count() + word.chars().count() + 1 > max_chars {
                        if self.y(y + 8) > bottom { break; }
                        self.legacy_text(label.x, y, &line, label.color, label.scale);
                        y += 12;
                        line.clear();
                    }
                    if !line.is_empty() { line.push(' '); }
                    line.push_str(&short_title(word, max_chars));
                }
                if self.y(y + 8) <= bottom && !line.is_empty() {
                    self.legacy_text(label.x, y, &line, label.color, label.scale);
                }
            } else {
                let x = if label.centered { label.x.saturating_sub(label.value.chars().count() * 3 * label.scale) } else { label.x };
                self.legacy_text(x,label.y,&label.value,label.color,label.scale);
            }
        }
    }
}

#[cfg(windows)]
#[repr(C)] struct DibHeader { size:u32,width:i32,height:i32,planes:u16,bit_count:u16,compression:u32,size_image:u32,xppm:i32,yppm:i32,used:u32,important:u32 }
#[cfg(windows)]
#[repr(C)] struct DibInfo { header:DibHeader, colors:[u32;1] }
#[cfg(windows)]
#[link(name="gdi32")]
unsafe extern "system" {
    fn CreateCompatibleDC(dc:*mut std::ffi::c_void)->*mut std::ffi::c_void;
    fn CreateDIBSection(dc:*mut std::ffi::c_void,info:*const DibInfo,usage:u32,bits:*mut *mut std::ffi::c_void,section:*mut std::ffi::c_void,offset:u32)->*mut std::ffi::c_void;
    fn SelectObject(dc:*mut std::ffi::c_void,obj:*mut std::ffi::c_void)->*mut std::ffi::c_void;
    fn CreateFontW(height:i32,width:i32,esc:i32,orient:i32,weight:i32,italic:u32,underline:u32,strike:u32,charset:u32,out_precision:u32,clip_precision:u32,quality:u32,pitch:u32,face:*const u16)->*mut std::ffi::c_void;
    fn SetBkMode(dc:*mut std::ffi::c_void,mode:i32)->i32;
    fn SetTextColor(dc:*mut std::ffi::c_void,color:u32)->u32;
    fn GetTextExtentPoint32W(dc:*mut std::ffi::c_void,text:*const u16,count:i32,size:*mut [i32;2])->i32;
    fn TextOutW(dc:*mut std::ffi::c_void,x:i32,y:i32,text:*const u16,count:i32)->i32;
    fn DeleteObject(obj:*mut std::ffi::c_void)->i32;
    fn DeleteDC(dc:*mut std::ffi::c_void)->i32;
}
#[cfg(windows)]
#[repr(C)]
struct TextRect { left: i32, top: i32, right: i32, bottom: i32 }

#[cfg(windows)]
#[link(name = "user32")]
unsafe extern "system" {
    fn DrawTextW(dc: *mut std::ffi::c_void, text: *const u16, count: i32, rect: *mut TextRect, format: u32) -> i32;
}

#[cfg(windows)]
fn draw_labels_gdi(c:&mut NativeCanvas<'_>){
    if c.labels.is_empty()||c.width==0||c.height==0{return;}
    unsafe {
        let dc=CreateCompatibleDC(std::ptr::null_mut()); if dc.is_null(){return;}
        let info=DibInfo{header:DibHeader{size:std::mem::size_of::<DibHeader>() as u32,width:c.width as i32,height:-(c.height as i32),planes:1,bit_count:32,compression:0,size_image:0,xppm:0,yppm:0,used:0,important:0},colors:[0]};
        let mut bits=std::ptr::null_mut(); let bitmap=CreateDIBSection(dc,&info,0,&mut bits,std::ptr::null_mut(),0);
        if bitmap.is_null()||bits.is_null(){DeleteDC(dc);return;}
        let old_bitmap=SelectObject(dc,bitmap);std::ptr::copy_nonoverlapping(c.dst.as_ptr(),bits as *mut u32,c.width*c.height);
        SetBkMode(dc,1);
        let face:Vec<u16>="Segoe UI".encode_utf16().chain(std::iter::once(0)).collect();
        let mut fonts:Vec<(usize,*mut std::ffi::c_void)>=Vec::new();
        for label in &c.labels {
            if label.value.is_empty() { continue; }
            let scale=label.scale.clamp(1,4);
            let font=if let Some((_,f))=fonts.iter().find(|(s,_)|*s==scale){*f}else{
                let px=(9.0*scale as f32*c.view.3 as f32/H as f32).round().max(12.0) as i32;
                let f=CreateFontW(-px,0,0,0,if scale>=2{600}else{450},0,0,0,1,0,0,5,0,face.as_ptr());fonts.push((scale,f));f
            };
            let old_font=SelectObject(dc,font);
            SetTextColor(dc,((label.color&0xFF)<<16)|(label.color&0xFF00)|((label.color>>16)&0xFF));
            let text:Vec<u16>=label.value.encode_utf16().collect();
            let x=(c.view.0 as f32+label.x as f32*c.view.2 as f32/W as f32).round() as i32;
            let y=(c.view.1 as f32+label.y as f32*c.view.3 as f32/H as f32).round() as i32;
            if let Some((right, bottom)) = label.bounds {
                const WORD_BREAK: u32 = 0x10;
                const EXTERNAL_LEADING: u32 = 0x200;
                const NO_PREFIX: u32 = 0x800;
                const EDIT_CONTROL: u32 = 0x2000;
                const END_ELLIPSIS: u32 = 0x8000;
                let mut rect = TextRect { left: x, top: y, right: right as i32, bottom: bottom as i32 };
                let flags = WORD_BREAK | EXTERNAL_LEADING | NO_PREFIX | EDIT_CONTROL;
                let mut measured = TextRect { left: x, top: y, right: right as i32, bottom: bottom as i32 };
                let measured_height = DrawTextW(dc, text.as_ptr(), text.len() as i32, &mut measured, flags | 0x400);
                rect.top += (rect.bottom - rect.top - measured_height).max(0) / 2;
                DrawTextW(dc, text.as_ptr(), text.len() as i32, &mut rect, flags | END_ELLIPSIS);
            } else {
                let mut size = [0i32; 2];
                if label.centered { GetTextExtentPoint32W(dc, text.as_ptr(), text.len() as i32, &mut size); }
                TextOutW(dc,x - size[0] / 2,y,text.as_ptr(),text.len() as i32);
            }
            SelectObject(dc,old_font);
        }
        std::ptr::copy_nonoverlapping(bits as *const u32,c.dst.as_mut_ptr(),c.width*c.height);
        for (_,font) in fonts{DeleteObject(font);}SelectObject(dc,old_bitmap);DeleteObject(bitmap);DeleteDC(dc);
    }
}

#[derive(Clone)]
pub struct Item { pub path: PathBuf, pub title: String, pub cover: Option<Artwork>, pub disc: Option<Artwork>, pub logo: Option<Artwork> }

pub fn scan(roots: &[PathBuf], covers_root: &Path, cd_root: &Path, artwork_root: &Path) -> Vec<Item> {
    load_items(scan_paths(roots), covers_root, cd_root, artwork_root)
}

pub fn scan_paths(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for root in roots { scan_dir(root, 0, &mut seen, &mut paths); }
    let cue_stems: HashSet<String> = paths.iter().filter(|p| ext(p) == "cue")
        .filter_map(|p| p.file_stem()).map(|s| s.to_string_lossy().to_lowercase()).collect();
    paths.retain(|p| !(ext(p) == "bin" && p.file_stem().map(|s| cue_stems.contains(&s.to_string_lossy().to_lowercase())).unwrap_or(false)));
    paths.sort_by_cached_key(|p| (p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default(), p.clone()));
    paths.truncate(MAX_ITEMS);
    paths
}

pub fn load_items(paths: Vec<PathBuf>, covers_root: &Path, cd_root: &Path, artwork_root: &Path) -> Vec<Item> {
    paths.into_iter().map(|path| {
        let title = disc_title(&path);
        let cover = load_cover_artwork(&path, covers_root);
        let disc = load_disc_artwork(&path, cd_root);
        let logo = managed_art_path(&path, artwork_root, crate::launchbox::ArtworkKind::Logo).and_then(|p| load_artwork(&p));
        Item { path, title, cover, disc, logo }
    }).collect()
}

pub fn migrate_artwork_names(paths: &[PathBuf], root: &Path) {
    for disc in paths {
        let title = disc_title(disc);
        let name = crate::launchbox::safe_name(&title);
        let mut changes = vec![(crate::boxart::legacy_generated_path(root, &title), crate::boxart::generated_path(root, &title))];
        for extension in ["png", "jpg", "jpeg"] {
            changes.push((root.join("CD").join(format!("{name}-disc.{extension}")), root.join("CD").join(format!("{name}.{extension}"))));
        }
        for (old, new) in changes {
            if !new.exists() && old.is_file() {
                if let Err(error) = std::fs::rename(&old, &new) { log::debug!("Cannot rename artwork {old:?}: {error}"); }
            }
        }
    }
}

pub fn managed_art_path(disc: &Path, root: &Path, kind: crate::launchbox::ArtworkKind) -> Option<PathBuf> {
    let folder = root.join(kind.folder());
    let title = disc_title(disc);
    if kind == crate::launchbox::ArtworkKind::Boxart {
        let path = crate::boxart::generated_path(root, &title);
        return path.is_file().then_some(path).or_else(|| {
            let old = crate::boxart::legacy_generated_path(root, &title);
            old.is_file().then_some(old)
        });
    }
    if let Some(path) = crate::screenscraper::preferred_gallery_image(&folder, kind, &title) { return Some(path); }
    let stem = disc.file_stem()?.to_string_lossy();
    ["png", "jpg", "jpeg"].into_iter().flat_map(|extension| {
        [folder.join(format!("{stem}.{extension}")), folder.join(kind.filename(&title, extension))]
    }).find(|path| path.is_file())
}

#[derive(Clone, Default)]
pub struct DiscDetails {
    pub logo: Option<Artwork>,
    pub boxart: Option<Artwork>,
    pub back: Option<Artwork>,
}

#[derive(Default)]
pub struct BootScene {
    pixels: Vec<u32>,
    size: (usize, usize),
    view: (usize, usize, usize, usize),
    title: String,
    synopsis: Option<String>,
    details: DiscDetails,
    preview: bool,
}

impl BootScene {
    pub fn render(&mut self, dst: &mut [u32], width: usize, height: usize,
        view: (usize, usize, usize, usize), title: &str, details: &DiscDetails,
        synopsis: Option<&str>, preview: bool, disc: Option<&Artwork>, frame: u64) {
        let same = |a: &Option<Artwork>, b: &Option<Artwork>| match (a, b) {
            (None, None) => true,
            (Some(a), Some(b)) => a.w == b.w && a.h == b.h && Arc::ptr_eq(&a.pixels, &b.pixels),
            _ => false,
        };
        if self.size != (width, height) || self.view != view || self.title != title
            || self.synopsis.as_deref() != synopsis || self.preview != preview
            || !same(&self.details.logo, &details.logo) || !same(&self.details.boxart, &details.boxart)
            || !same(&self.details.back, &details.back) {
            self.pixels.resize(width * height, 0);
            self.pixels.fill(0xFF0B1523);
            render_boot_details(&mut self.pixels, width, height, view, title, details, synopsis);
            if preview { render_preview_actions(&mut self.pixels, width, height, view); }
            self.size = (width, height);
            self.view = view;
            self.title = title.to_owned();
            self.synopsis = synopsis.map(str::to_owned);
            self.details = details.clone();
            self.preview = preview;
        }
        dst.copy_from_slice(&self.pixels);
        let cx = view.0 + 520 * view.2 / W;
        let cy = view.1 + 398 * view.3 / H;
        let size = (176 * view.2 / W).min(176 * view.3 / H).max(1);
        blit_rotated_into(dst, width, height, disc.unwrap_or(&MISSING_DISC), cx as i32, cy as i32, size as i32, frame as f32 * 0.055);
    }
}

impl DiscDetails {
    pub fn load(disc: &Path, root: &Path) -> Self {
        use crate::launchbox::ArtworkKind;
        let image = |kind| managed_art_path(disc, root, kind).and_then(|path| load_artwork(&path));
        Self { logo: image(ArtworkKind::Logo), boxart: image(ArtworkKind::Boxart), back: image(ArtworkKind::Back) }
    }
}

pub fn disc_title(path: &Path) -> String {
    path.file_stem().map(|stem| stem.to_string_lossy().replace('_', " ")).unwrap_or_else(|| "CD-i Disc".into())
}

fn ext(p: &Path) -> String { p.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase() }

fn scan_dir(dir: &Path, depth: usize, seen: &mut HashSet<PathBuf>, out: &mut Vec<PathBuf>) {
    if depth > 32 || out.len() >= MAX_ITEMS { return; }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        if out.len() >= MAX_ITEMS { break; }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            if !matches!(name.as_str(), ".git" | "target" | "vendor" | "node_modules" | "bios" | "dvc") {
                scan_dir(&path, depth + 1, seen, out);
            }
        } else if ft.is_file() && matches!(ext(&path).as_str(), "chd" | "iso" | "cue" | "bin") {
            let key = std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
            if seen.insert(key) { out.push(path); }
        }
    }
}

fn art_in_root(disc: &Path, root: &Path, cover: bool) -> Option<PathBuf> {
    let stem = disc.file_stem()?.to_string_lossy();
    let downloaded_name = crate::launchbox::safe_name(&disc_title(disc));
    let suffixes: &[&str] = if cover { &["", "-cover", "_cover"] } else { &["", "-disc", "-cd", "_disc"] };
    for name in [stem.as_ref(), &downloaded_name] {
        for suffix in suffixes {
            for ext in ["png", "jpg", "jpeg"] {
                let candidate = root.join(format!("{name}{suffix}.{ext}"));
                if candidate.is_file() { return Some(candidate); }
            }
        }
    }
    None
}

pub fn has_disc_image(disc: &Path, root: &Path, cover: bool) -> bool {
    let kind = if cover { crate::launchbox::ArtworkKind::Cover } else { crate::launchbox::ArtworkKind::Disc };
    crate::screenscraper::preferred_gallery_image(root, kind, &disc_title(disc)).is_some()
        || art_in_root(disc, root, cover).is_some()
}

pub fn cover_artwork_path(disc: &Path, root: &Path) -> Option<PathBuf> {
    crate::screenscraper::preferred_gallery_image(root, crate::launchbox::ArtworkKind::Cover, &disc_title(disc))
        .or_else(|| art_in_root(disc, root, true))
}

pub fn load_cover_artwork(disc: &Path, root: &Path) -> Option<Artwork> {
    crate::screenscraper::preferred_gallery_image(root, crate::launchbox::ArtworkKind::Cover, &disc_title(disc))
        .and_then(|path| load_artwork(&path))
        .or_else(|| art_in_root(disc, root, true).and_then(|p| load_artwork(&p)))
}

pub fn load_disc_artwork(disc: &Path, root: &Path) -> Option<Artwork> {
    let kind = crate::launchbox::ArtworkKind::Disc;
    crate::screenscraper::preferred_gallery_image(root, kind, &disc_title(disc))
        .and_then(|path| load_artwork(&path))
        .or_else(|| art_in_root(disc, root, false).and_then(|p| load_artwork(&p)))
        .map(|image| crate::disc_art::normalize(&image))
}

pub fn load_artwork(path: &Path) -> Option<Artwork> {
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    decode_png(file).or_else(|| decode_wic(path))
}

#[cfg(windows)]
fn decode_wic(path: &Path) -> Option<Artwork> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{GENERIC_READ, RPC_E_CHANGED_MODE};
    use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
    struct ComInit;
    impl Drop for ComInit { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
    unsafe {
        let init = CoInitializeEx(None, COINIT_MULTITHREADED);
        // The Windows UI thread may already own a single-threaded apartment.
        // WIC works in that apartment. Only balance initializations we own.
        let _com = if init.is_ok() {
            Some(ComInit)
        } else if init == RPC_E_CHANGED_MODE {
            None
        } else {
            return None;
        };
        let factory: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER).ok()?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let decoder = factory.CreateDecoderFromFilename(PCWSTR(wide.as_ptr()), None, GENERIC_READ, WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let mut width = 0;
        let mut height = 0;
        frame.GetSize(&mut width, &mut height).ok()?;
        let len = (width as usize).checked_mul(height as usize)?.checked_mul(4)?;
        if width == 0 || height == 0 || len > 128 * 1024 * 1024 { return None; }
        let converter = factory.CreateFormatConverter().ok()?;
        converter.Initialize(&frame, &GUID_WICPixelFormat32bppRGBA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        let mut data = vec![0u8; len];
        converter.CopyPixels(std::ptr::null(), width.checked_mul(4)?, &mut data).ok()?;
        let pixels = data.chunks_exact(4).map(|p| ((p[3] as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32).collect();
        Some(Artwork { w: width as usize, h: height as usize, pixels })
    }
}

#[cfg(not(windows))]
fn decode_wic(_path: &Path) -> Option<Artwork> { None }

pub fn load_png_bytes(bytes: &[u8]) -> Option<Artwork> {
    decode_png(std::io::Cursor::new(bytes))
}

static MISSING_COVER: LazyLock<Artwork> = LazyLock::new(|| {
    load_png_bytes(include_bytes!("../assets/cover_missing.png")).expect("embedded cover placeholder")
});
static MISSING_DISC: LazyLock<Artwork> = LazyLock::new(|| {
    load_png_bytes(include_bytes!("../assets/disc_missing.png")).expect("embedded disc placeholder")
});

fn decode_png<R: std::io::BufRead + std::io::Seek>(file: R) -> Option<Artwork> {
    let mut decoder = png::Decoder::new(file);
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().ok()?;
    let mut data = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut data).ok()?;
    if info.color_type != png::ColorType::Rgba || info.width == 0 || info.height == 0 { return None; }
    data.truncate(info.buffer_size());
    let pixels = data.chunks_exact(4).map(|p| ((p[3] as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32).collect();
    Some(Artwork { w: info.width as usize, h: info.height as usize, pixels })
}

pub fn render(dst: &mut [u32], items: &[Item], selected: usize, art_size: u8) {
    for y in 0..H { for x in 0..W {
        let c = 9 + (y * 15 / H) as u32;
        dst[y * W + x] = 0xFF00_0000 | (c << 16) | ((c + 13) << 8) | (c + 29);
    }}
    rect(dst, 0, 0, W, 54, 0xFF10182B);
    text(dst, 24, 17, "CDI DISC LIBRARY", 0xFFF4E7C5, 2);
    text(dst, 590, 22, &format!("{} TITLES", items.len()), 0xFF90A4B8, 1);
    render_size_slider(dst, art_size);
    if items.is_empty() {
        text(dst, 205, 210, "NO DISC IMAGES FOUND", 0xFFFFD27A, 2);
        text(dst, 135, 250, "ADD A DISC FOLDER FROM SETTINGS TO BEGIN", 0xFFE1E5ED, 1);
        text(dst, 180, 330, "PRESS CTRL O TO BROWSE FOR A DISC", 0xFF90A4B8, 1);
    } else {
        let layout = layout(art_size);
        let per_page = layout.visible_count();
        let page_start = (selected / per_page) * per_page;
        for slot in 0..per_page {
            let i = page_start + slot;
            if i >= items.len() { break; }
            let col = slot % layout.columns; let row = slot / layout.columns;
            let x = layout.left + col * (layout.cell_w + layout.gap);
            let y = layout.top + row * (layout.cell_h + layout.gap);
            let active = i == selected;
            let art_x = x + 3;
            let art_y = y + 2;
            let art_w = layout.cell_w.saturating_sub(6);
            let art_h = layout.cell_h.saturating_sub(42);
            blit_cover_fit(dst, W, H, items[i].cover.as_ref().unwrap_or(&MISSING_COVER), art_x, art_y, art_w, art_h);
            let title: String = items[i].title.chars().take(24).collect();
            text(dst, x + 4, y + layout.cell_h - 30, &title, if active { 0xFFFFD98A } else { 0xFFE6EAF0 }, 1);
            let format = ext(&items[i].path).to_ascii_uppercase();
            text(dst, x + 4, y + layout.cell_h - 15, &format, 0xFF93A7BC, 1);
            if active { rect(dst, x + 4, y + layout.cell_h - 3, layout.cell_w.saturating_sub(8), 2, 0xFFFFC45B); }
        }
        let pages = (items.len() + per_page - 1) / per_page;
        text(dst, 23, 512, "ARROWS OR GAMEPAD D PAD / STICK: MOVE     ENTER / A: PLAY", 0xFFD5DCE7, 1);
        text(dst, 23, 530, "LEFT CLICK: DETAILS     WHEEL: SCROLL     ESC: QUIT", 0xFFD5DCE7, 1);
        text(dst, 717, 544, &format!("{}/{}", selected / per_page + 1, pages), 0xFFFFD98A, 1);
    }
}

#[derive(Default)]
pub struct Search {
    pub active_bios: Option<(String,String)>,
    pub pointer: Option<(i32, i32)>,
    pub query: String,
    pub focused: bool,
    pub caret: usize,
    pub select_all: bool,
    matches: Vec<usize>,
    pub collection: Collection,
}

fn search_key(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).filter(|c| c.is_alphanumeric()).collect()
}

impl Search {
    pub fn rebuild(&mut self, items: &[Item]) {
        self.matches.clear();
        if self.is_unfiltered() { return; }
        let terms: Vec<_> = self.query.split_whitespace().map(search_key).filter(|term| !term.is_empty()).collect();
        self.matches.extend(items.iter().enumerate().filter_map(|(index, item)| {
            if !self.collection.includes(index) { return None; }
            if terms.is_empty() { return Some(index); }
            let title = search_key(&item.title);
            let file = search_key(&item.path.file_name().unwrap_or_default().to_string_lossy());
            terms.iter().all(|term| title.contains(term) || file.contains(term)).then_some(index)
        }));
    }

    fn is_unfiltered(&self) -> bool {
        self.query.is_empty() && self.collection.category == Category::All
    }

    pub fn len(&self, items: &[Item]) -> usize {
        if self.is_unfiltered() { items.len() } else { self.matches.len() }
    }

    pub fn get<'a>(&self, items: &'a [Item], index: usize) -> Option<&'a Item> {
        items.get(if self.is_unfiltered() { index } else { *self.matches.get(index)? })
    }

    pub fn position(&self, items: &[Item], path: &Path) -> Option<usize> {
        (0..self.len(items)).find(|&index| self.get(items, index).is_some_and(|item| item.path == path))
    }

    pub fn insert(&mut self, text: &str) -> bool {
        let room = if self.select_all { 120 } else { 120usize.saturating_sub(self.query.chars().count()) };
        let value: String = text.chars().filter(|ch| !ch.is_control()).take(room).collect();
        if value.is_empty() { return false; }
        if self.select_all { self.clear(); }
        self.query.insert_str(self.caret, &value);
        self.caret += value.len();
        true
    }

    pub fn clear(&mut self) {
        self.query.clear();
        self.caret = 0;
        self.select_all = false;
    }

    pub fn move_caret(&mut self, right: bool) {
        if self.select_all {
            self.caret = if right { self.query.len() } else { 0 };
        } else if right {
            self.caret += self.query[self.caret..].chars().next().map_or(0, char::len_utf8);
        } else {
            self.caret = self.query[..self.caret].char_indices().next_back().map_or(0, |(index, _)| index);
        }
        self.select_all = false;
    }

    pub fn delete(&mut self, backwards: bool) -> bool {
        if self.query.is_empty() { return false; }
        if self.select_all { self.clear(); return true; }
        let original = self.caret;
        self.move_caret(!backwards);
        let end = original.max(self.caret);
        self.caret = original.min(self.caret);
        self.query.replace_range(self.caret..end, "");
        end != self.caret
    }
}

pub fn search_hit(x: i32, y: i32) -> Option<bool> {
    ((455..710).contains(&x) && (45..73).contains(&y)).then_some(x >= 681)
}

const TAB_LEFT: i32 = 14;
const TAB_WIDTH: i32 = 92;
const TAB_TOP: i32 = 76;
const TAB_HEIGHT: i32 = 27;
const MISSING_LEFT: i32 = 50;
const MISSING_RIGHT: i32 = 718;
const MISSING_TOP: i32 = 111;
const MISSING_BOTTOM: i32 = 445;
const MISSING_FIRST_ROW: i32 = 158;
const MISSING_ROW_HEIGHT: i32 = 25;

pub fn category_hit(x: i32, y: i32) -> Option<Category> {
    if x < TAB_LEFT || !(TAB_TOP..TAB_TOP + TAB_HEIGHT).contains(&y) { return None; }
    Category::TABS.get(((x - TAB_LEFT) / TAB_WIDTH) as usize).copied()
}

pub fn collection_count_hit(x: i32, y: i32) -> bool {
    (476..753).contains(&x) && (H as i32 - 94..H as i32 - 62).contains(&y)
}

pub fn missing_close_hit(x: i32, y: i32) -> bool {
    (647..704).contains(&x) && (120..148).contains(&y)
}

pub fn missing_row_hit(x: i32, y: i32) -> Option<usize> {
    if !(MISSING_LEFT..MISSING_RIGHT).contains(&x) || y < MISSING_FIRST_ROW { return None; }
    let row = ((y - MISSING_FIRST_ROW) / MISSING_ROW_HEIGHT) as usize;
    (row < MISSING_ROWS).then_some(row)
}

fn render_missing(c: &mut NativeCanvas<'_>, collection: &Collection, offset: usize) {
    c.labels.retain(|label| label.y < MISSING_TOP as usize || label.y >= MISSING_BOTTOM as usize);
    c.round_rect(MISSING_LEFT as usize, MISSING_TOP as usize, (MISSING_RIGHT - MISSING_LEFT) as usize,
        (MISSING_BOTTOM - MISSING_TOP) as usize, 9, 0xFF365573);
    c.round_rect(MISSING_LEFT as usize + 1, MISSING_TOP as usize + 1, (MISSING_RIGHT - MISSING_LEFT) as usize - 2,
        (MISSING_BOTTOM - MISSING_TOP) as usize - 2, 8, 0xFF142337);
    c.text(66, 124, &format!("Missing titles · {}", collection.category.label()), 0xFFF1F5FA, 2);
    c.round_rect(647, 120, 57, 28, 5, 0xFF243C55);
    c.centered_text(675, 127, "Close", 0xFFE5F1FF, 1);
    if collection.missing_count() == 0 {
        c.centered_text(W / 2, 250, "No missing titles in this category.", 0xFFD5DFEA, 1);
        return;
    }
    for row in 0..MISSING_ROWS {
        let Some(title) = collection.missing_title(offset + row) else { break; };
        let y = MISSING_FIRST_ROW as usize + row * MISSING_ROW_HEIGHT as usize;
        if row % 2 == 0 { c.rect(63, y, 642, MISSING_ROW_HEIGHT as usize, 0xFF192B40); }
        c.text(72, y + 5, &short_title(&title.name, 84), 0xFFB9DEFF, 1);
    }
    let end = (offset + MISSING_ROWS).min(collection.missing_count());
    c.text(68, 420, &format!("{}–{} of {}  ·  Scroll for more  ·  Click a title to open its source",
        offset + 1, end, collection.missing_count()), 0xFFA3B6CB, 1);
}

pub fn render_native(dst: &mut [u32], width: usize, height: usize, view: (usize,usize,usize,usize), items: &[Item], selected: usize, art_size: u8, toolbar_visible: bool, mode: DisplayMode, popup: Option<ToolbarPopup>, optical_disc: Option<char>, search: &Search, progress: Option<&crate::launchbox::LookupProgress>) {
    let mut c=NativeCanvas::new(dst,width,height,view);
    for y in 0..H { let t=y as u32*10/H as u32; let color=0xFF000000|((10+t)<<16)|((20+t)<<8)|(34+t*2); c.rect(0,y,W,1,color); }
    c.round_rect(12,43,W-24,32,8,0xFF101D2C);
    c.text(25,48,"CD-i Library",0xFFF1F5FA,2);
    c.surface(455,45,255,28,8,0xFF1D2D41,0xFF172537,if search.focused{0xFF62C1FF}else{0xFF344A62});
    c.icon(6,465,51,14,0xFFAAC3DD);
    if search.query.is_empty() {
        c.text(486,53,"Search library...",0xFFA3B6CB,1);
    } else {
        let mut chars: Vec<_> = search.query.chars().collect();
        let caret = search.query[..search.caret].chars().count();
        let start = if search.focused { caret.saturating_sub(21) } else { 0 };
        if search.focused && !search.select_all { chars.insert(caret, '|'); }
        let shown: String = chars.into_iter().skip(start).take(23).collect();
        if search.focused && search.select_all { c.round_rect(483,50,194,18,3,0xFF324F72); }
        c.text(486,53,&shown,0xFFF1F5FA,1);
        c.text(689,52,"×",0xFFD6E2EF,1);
    }
    c.line(14, 104, W - 28, 0xFF26374A);
    for (index, category) in Category::TABS.into_iter().enumerate() {
        let x = TAB_LEFT as usize + index * TAB_WIDTH as usize;
        let active = category == search.collection.category;
        let hover=search.pointer.is_some_and(|(px,py)| category_hit(px,py)==Some(category));
        if active || hover {
            c.surface(x+1,TAB_TOP as usize,TAB_WIDTH as usize-2,TAB_HEIGHT as usize,7,
                if active{0xFF2A4863}else{0xFF22394E},if active{0xFF203A53}else{0xFF192B3D},
                if active{0xFF64BFF9}else{0xFF36526D});
        }
        c.icon([7,1,8,9,10,11,12,13][index],x+8,82,12,if active{0xFFAEE0FF}else{0xFFA5BDD5});
        c.centered_text(x+TAB_WIDTH as usize/2+7,84,category.label(),if active{0xFFE6F4FF}else{0xFFB9C9DB},1);
    }
    c.rect(0,H-100,W,100,0xFF0D1725);
    c.rect(0,H-100,W,1,0xFF26374A);
    if let Some(progress) = progress {
        let status = if progress.finished {
            format!("{} · {} images saved · {} discs missing media", progress.phase, progress.saved, progress.missing)
        } else {
            format!("{} · {} of {} discs · {} images saved", progress.phase, progress.checked, progress.total, progress.saved)
        };
        c.text(18,H-94,&short_title(&status, 70),0xFFD5DFEA,1);
        c.round_rect(18,H-75,180,3,1,0xFF33465A);
        let filled = if progress.total == 0 { 0 } else { progress.checked.min(progress.total) * 180 / progress.total };
        c.round_rect(18,H-75,filled,3,1,0xFF57AEF5);
        if !progress.current.is_empty() {
            c.text(213,H-79,&short_title(&progress.current, 42),0xFF9AAFC4,1);
        }
    }
    c.round_rect(476, H - 94, 277, 32, 6, 0xFF203852);
    let owned = search.collection.owned_count();
    let missing = search.collection.missing_count();
    let discs = if owned == 1 { "disc" } else { "discs" };
    let titles = if missing == 1 { "title" } else { "titles" };
    let collection_count = format!("{owned} {discs} in collection  ·  {missing} {titles} missing");
    c.centered_text(615, H - 85, &collection_count, 0xFFE6F4FF, 1);
    c.text(18,H-48,"ARTWORK SIZE",0xFFB7C5D4,1);
    let track_x=125; let track_w=145; let track_y=H-35;
    c.surface(track_x,track_y,track_w,4,2,0xFF24394D,0xFF182737,0xFF304357);
    let value=art_size.clamp(ART_SIZE_MIN,ART_SIZE_MAX);
    let filled=(value-ART_SIZE_MIN) as usize*track_w/(ART_SIZE_MAX-ART_SIZE_MIN) as usize;
    if filled>0 {c.surface(track_x,track_y,filled,4,2,0xFF8AD4FF,0xFF439BD4,0xFF62B9F0);}
    c.surface(track_x+filled-5,track_y.saturating_sub(4),11,12,6,0xFFDDF2FF,0xFF79B9E3,0xFFA6DBFF);
    let l = layout(value);
    c.text(282,H-42,&format!("{} × {}", l.columns, l.rows),0xFFD5DFEA,1);
    let count = search.len(items);
    if let Some(item) = search.get(items, selected) {
        c.text(338, H - 42, &short_title(&item.title, 46), 0xFFB7C5D4, 1);
    }
    let count_label = if search.query.is_empty() { format!("{count} ITEMS") }
        else { format!("{count} OF {}", search.collection.owned_count()) };
    c.text(660,H-42,&count_label,0xFF9AAFC4,1);
    let per_page = l.visible_count();
    if count == 0 && !search.query.is_empty() {
        c.centered_text(W / 2, 242, "No discs match your search", 0xFFF1F5FA, 2);
    } else if count == 0 && search.collection.category != Category::All {
        c.centered_text(W / 2, 242, "No discs in this category", 0xFFF1F5FA, 2);
    } else if count == 0 {
        c.text(205,210,"NO DISC IMAGES FOUND",0xFFF1F5FA,2);
        c.text(135,250,"ADD A DISC FOLDER FROM SETTINGS TO BEGIN",0xFF9AAFC4,1);
        c.text(180,290,"OR CHOOSE OPEN DISC",0xFF78BDF5,1);
    } else {
        let page_start=(selected/per_page)*per_page;
        for slot in 0..per_page {
            let i=page_start+slot; let Some(item) = search.get(items, i) else { break; };
            let col=slot%l.columns; let row=slot/l.columns;
            let x=l.left+col*(l.cell_w+l.gap); let y=l.top+row*(l.cell_h+l.gap);
            let art_x=x+4; let art_y=y+2; let art_w=l.cell_w.saturating_sub(8); let art_h=l.cell_h.saturating_sub(31);
            c.cover_fit(item.cover.as_ref().unwrap_or(&MISSING_COVER), art_x, art_y, art_w, art_h);
            let active = i == selected;
            if let Some(logo) = &item.logo {
                c.art_fit(logo, x + 3, y + l.cell_h - 28, l.cell_w - 6, 22);
            } else {
                let title_cap = (l.cell_w.saturating_sub(6) / 6).max(1);
                let (one, two) = wrap_title(&item.title, title_cap);
                let color = if active { 0xFF80C4FF } else { 0xFFE8EEF5 };
                for (line, offset) in [(&one, 26), (&two, 14)] {
                    let inset = l.cell_w.saturating_sub(line.chars().count() * 6) / 2;
                    c.text(x + inset, y + l.cell_h - offset, line, color, 1);
                }
            }
            if active { c.round_rect(x+3,y+l.cell_h-4,l.cell_w.saturating_sub(6),2,1,0xFF56B1F7); }
        }
        let pages=count.div_ceil(per_page);
        c.text(18,H-18,"LEFT CLICK: DISC DETAILS    D-PAD / STICK: MOVE",0xFF778DA5,1);
        c.text(686,H-18,&format!("{}/{}",selected/per_page+1,pages),0xFFB7C5D4,1);
    }
    if let Some(offset) = search.collection.missing_offset {
        render_missing(&mut c, &search.collection, offset);
    }
    // Draw popovers over the grid so they remain legible when they extend below the toolbar.
    if toolbar_visible { render_toolbar_native(&mut c,mode,popup,optical_disc,search.pointer,search.active_bios.as_ref().map(|(model,file)| (model.as_str(),file.as_str()))); }
}

fn short_title(title: &str, width: usize) -> String {
    if title.chars().count() <= width { return title.to_owned(); }
    title.chars().take(width.saturating_sub(1)).chain(['…']).collect()
}

fn wrap_title(title: &str, width: usize) -> (String, String) {
    let Some((limit, _)) = title.char_indices().nth(width) else {
        return (title.to_owned(), String::new());
    };
    let split = title[..limit].rfind(' ').filter(|&i| i > 0).unwrap_or(limit);
    (title[..split].to_owned(), short_title(title[split..].trim_start(), width))
}

fn dock_buttons(optical: Option<char>) -> Vec<(ToolbarAction, usize, usize, &'static str, u8)> {
    let mut buttons = Vec::new();
    let mut x = 20;
    if let Some(letter) = optical { buttons.push((ToolbarAction::PlayDisc(letter), x, 70, "Play Disc", 4)); x += 74; }
    buttons.push((ToolbarAction::OpenDisc, x, 76, "Open Disc", 0));
    buttons.push((ToolbarAction::Controller, x + 80, 80, "Controller", 1));
    buttons.push((ToolbarAction::Start220, 286, 78, "Start 220", 5));
    buttons.push((ToolbarAction::Start490, 370, 78, "Start 490", 5));
    buttons.extend([(ToolbarAction::Settings, 564, 65, "Settings", 2),
        (ToolbarAction::About, 633, 58, "About", 3), (ToolbarAction::Exit, 695, 59, "Exit", 5)]);
    buttons
}

fn render_toolbar_native(c:&mut NativeCanvas<'_>,mode:DisplayMode,popup:Option<ToolbarPopup>, optical_disc: Option<char>, pointer: Option<(i32,i32)>, bios: Option<(&str,&str)>){
    c.rect(0,0,W,TOOLBAR_HEIGHT,0xFF0C1726);
    c.line(0,TOOLBAR_HEIGHT-1,W,0xFF26374A);
    c.surface(14,3,if optical_disc.is_some(){250}else{176},36,10,0xFF1D2E42,0xFF142235,0xFF304459);
    c.surface(558,3,202,36,10,0xFF1D2E42,0xFF142235,0xFF304459);
    c.surface(280,3,174,36,10,0xFF1D2E42,0xFF142235,0xFF405D78);
    let hovered=pointer.and_then(|(x,y)| if x>=0 && y>=0 { toolbar_hit(x as usize,y as usize,popup,optical_disc) } else {None});
    for (action,x,w,label,icon) in dock_buttons(optical_disc) {
        let active=matches!((action,popup),(ToolbarAction::Settings,Some(ToolbarPopup::Settings))|(ToolbarAction::About,Some(ToolbarPopup::About)));
        let focus=active || hovered==Some(action);
        let exit=action==ToolbarAction::Exit;
        let (top,bottom,border)=if focus {
            if exit {(0xFF50313F,0xFF382632,0xFFF19CAA)} else {(0xFF2D4A64,0xFF233C54,0xFF62C1FF)}
        } else {(0xFF203247,0xFF18283B,0xFF2D435B)};
        if !exit || focus { c.surface(x,5,w,32,7,top,bottom,border); }
        c.icon(icon,x+w/2-7,8,14,if focus{0xFFA9DEFF}else{0xFFB0C4D9});
        c.centered_text(x+w/2,25,label,if exit && focus{0xFFFFCAD4}else{0xFFE2ECF6},1);
    }
    let _=mode;
    match popup {
        Some(ToolbarPopup::Settings) => {
            // Paint any queued grid text before the opaque menu covers it.
            c.flush_labels();
            c.surface(SETTINGS_X, SETTINGS_Y, SETTINGS_WIDTH, SETTINGS_HEADER + SETTINGS_ITEMS.len() * SETTINGS_ROW + 12, 9, 0xFF1F3046, 0xFF17263A, 0xFF3D536C);
            for (row, (label, action)) in SETTINGS_ITEMS.iter().enumerate() {
                let checked = bios.is_some_and(|(model,_)| match action {
                    ToolbarAction::Bios220 => model.contains("220"),
                    ToolbarAction::Bios490 => model.contains("490"),
                    _ => false,
                });
                let value=if checked { format!("✓  {label}") } else if row>=4 { format!("    {label}") } else { (*label).into() };
                c.text(SETTINGS_X + 13, SETTINGS_Y + SETTINGS_HEADER + 8 + row * SETTINGS_ROW, &value, 0xFFE6EDF5, 1);
            }
        }
        Some(ToolbarPopup::About) => {
            for pixel in &mut *c.dst {
                *pixel = (*pixel & 0xFF000000) | ((*pixel & 0x00FCFCFC) >> 2);
            }
            c.labels.retain_mut(|label| {
                let width = label.value.chars().count() * 9 * label.scale;
                let overlaps = label.x < 635 && label.x + width > 133 && label.y < 415 && label.y + 16 > 145;
                label.color = (label.color & 0xFF000000) | ((label.color & 0x00FCFCFC) >> 2);
                !overlaps
            });
            c.round_rect(133, 145, 502, 270, 10, 0xFF17263A);
            c.text(157, 165, "About OpenCDi", 0xFFF1F5FA, 2);
            for (row, line) in [
                "OpenCDi is a modern, standalone, open-source Philips CD-i",
                "emulator written in Rust. The goal of the project is to accurately",
                "preserve and run the software and multimedia library of the",
                "Philips CD-i system with a clean, modular, and fast architecture.",
            ].iter().enumerate() {
                c.text(157, 207 + row * 17, line, 0xFFE6EDF5, 1);
            }
            let version=format!("Version {} · Build {}",env!("CARGO_PKG_VERSION"),env!("OPENCDI_BUILD_ID"));
            for (row, line) in ["Emulator Core by sigsantilio and Arnold de Ruiter", "User Interface by sionisaac", version.as_str()].iter().enumerate() {
                c.text(157, 295 + row * 19, line, 0xFFE6EDF5, 1);
            }
            c.round_rect(535, 365, 76, 30, 6, 0xFF243B55);
            c.text(557, 373, "Close", 0xFFF1F5FA, 1);
        }
        None => {}
    }
}

pub fn about_close_hit(x: usize, y: usize) -> bool {
    (535..611).contains(&x) && (365..395).contains(&y)
}

pub fn toolbar_popup_hit(x: usize, y: usize, popup: Option<ToolbarPopup>) -> bool {
    match popup {
        Some(ToolbarPopup::Settings) => (SETTINGS_X..SETTINGS_X + SETTINGS_WIDTH).contains(&x)
            && (SETTINGS_Y..SETTINGS_Y + SETTINGS_HEADER + SETTINGS_ITEMS.len() * SETTINGS_ROW + 12).contains(&y),
        _ => false,
    }
}

pub fn draw_covers_high_res(dst: &mut [u32], dst_w: usize, dst_h: usize, view: (usize, usize, usize, usize), items: &[Item], selected: usize, art_size: u8) {
    let (vx, vy, vw, vh) = view;
    if vw == 0 || vh == 0 { return; }
    let layout = layout(art_size);
    let per_page = layout.visible_count();
    let page_start = (selected / per_page) * per_page;
    for slot in 0..per_page {
        let index = page_start + slot;
        if index >= items.len() { break; }
        let art = items[index].cover.as_ref().unwrap_or(&MISSING_COVER);
        let col = slot % layout.columns; let row = slot / layout.columns;
        let x = layout.left + col * (layout.cell_w + layout.gap) + 3;
        let y = layout.top + row * (layout.cell_h + layout.gap) + 2;
        let w = layout.cell_w.saturating_sub(6);
        let h = layout.cell_h.saturating_sub(42);
        let sx = |n: usize| vx + n * vw / W;
        let sy = |n: usize| vy + n * vh / H;
        blit_cover_fit(dst, dst_w, dst_h, art, sx(x), sy(y), (x + w) * vw / W + vx - sx(x), (y + h) * vh / H + vy - sy(y));
    }
}

const CASE_WIDTH: usize = 668;
const CASE_HEIGHT: usize = 612;

static COVER_FRAME: LazyLock<Artwork> = LazyLock::new(|| {
    let template = load_png_bytes(include_bytes!("../assets/boxart_case.png"))
        .expect("embedded jewel case");
    let mut pixels = vec![0; CASE_WIDTH * CASE_HEIGHT];
    for y in 0..CASE_HEIGHT {
        for x in 0..CASE_WIDTH {
            let edge = x.min(CASE_WIDTH - 1 - x).min(y).min(CASE_HEIGHT - 1 - y);
            let color = match edge {
                0 => 0x99848d96,
                1 => 0xffd7dbe0,
                2 => 0xfff1f3f5,
                3 => 0xff838c96,
                4 => 0xffc0c6cd,
                _ => 0xff22252a,
            };
            pixels[y * CASE_WIDTH + x] = color;
        }
    }
    for y in 6..606 {
        pixels[y * CASE_WIDTH + 62..y * CASE_WIDTH + 662].fill(0);
        for x in 6..56 {
            pixels[y * CASE_WIDTH + x] = sample_bilinear(&template,
                13.0 + (x - 6) as f32 * 30.0 / 50.0,
                12.0 + (y - 6) as f32 * 430.0 / 600.0) | 0xff000000;
        }
        pixels[y * CASE_WIDTH + 57] = 0xffe4e7ea;
        pixels[y * CASE_WIDTH + 59] = 0xff747e88;
        pixels[y * CASE_WIDTH + 60] = 0xffc0c5ca;
    }
    Artwork { w: CASE_WIDTH, h: CASE_HEIGHT, pixels: pixels.into() }
});

fn blit_cover_fit(
    dst: &mut [u32], dst_w: usize, dst_h: usize, cover: &Artwork,
    x: usize, y: usize, w: usize, h: usize,
) {
    if w == 0 || h == 0 { return; }
    let scale = (w as f32 / CASE_WIDTH as f32).min(h as f32 / CASE_HEIGHT as f32);
    let frame_w = (CASE_WIDTH as f32 * scale).max(1.0) as usize;
    let frame_h = (CASE_HEIGHT as f32 * scale).max(1.0) as usize;
    let left = x + (w - frame_w) / 2;
    let top = y + (h - frame_h) / 2;
    let inset_x = left + 62 * frame_w / CASE_WIDTH;
    let inset_y = top + 6 * frame_h / CASE_HEIGHT;
    let inset_w = 600 * frame_w / CASE_WIDTH;
    let inset_h = 600 * frame_h / CASE_HEIGHT;
    blit_fit_scaled(dst, dst_w, dst_h, cover, inset_x, inset_y, inset_w, inset_h);
    blit_fit_scaled(dst, dst_w, dst_h, &COVER_FRAME, left, top, frame_w, frame_h);
}

fn blit_fit_scaled(dst: &mut [u32], dst_w: usize, dst_h: usize, art: &Artwork, x: usize, y: usize, w: usize, h: usize) {
    if art.w == 0 || art.h == 0 || w == 0 || h == 0 { return; }
    let scale = (w as f32 / art.w as f32).min(h as f32 / art.h as f32);
    let rw = (art.w as f32 * scale).max(1.0) as usize; let rh = (art.h as f32 * scale).max(1.0) as usize;
    let ox = x + (w - rw) / 2; let oy = y + (h - rh) / 2;
    for dy in 0..rh {
        let py = oy + dy; if py >= dst_h { continue; }
        for dx in 0..rw {
            let px = ox + dx; if px >= dst_w { continue; }
            let color = sample_bilinear(art, (dx as f32 + 0.5) * art.w as f32 / rw as f32 - 0.5, (dy as f32 + 0.5) * art.h as f32 / rh as f32 - 0.5);
            let at = py * dst_w + px;
            if at < dst.len() { dst[at] = blend(color, dst[at], color >> 24); }
        }
    }
}

fn render_size_slider(dst: &mut [u32], art_size: u8) {
    let value = art_size.clamp(ART_SIZE_MIN, ART_SIZE_MAX);
    rect(dst, 0, TOOLBAR_HEIGHT, W, 40, 0xFF121E2D);
    text(dst, 20, 67, "ARTWORK SIZE", 0xFFBBC8D6, 1);
    let track_x = 184usize;
    let track_w = 430usize;
    let track_y = 72usize;
    rect(dst, track_x, track_y, track_w, 4, 0xFF3A4B5F);
    let t = (value - ART_SIZE_MIN) as usize;
    let filled = t * track_w / (ART_SIZE_MAX - ART_SIZE_MIN) as usize;
    rect(dst, track_x, track_y, filled, 4, 0xFFF5BD61);
    let knob_x = track_x + filled;
    rect(dst, knob_x.saturating_sub(5), track_y.saturating_sub(5), 11, 14, 0xFFFFD78E);
    text(dst, 635, 67, &format!("{}%", value), 0xFFFFD78E, 1);
}

pub fn size_slider_hit(x: usize, y: usize) -> bool { (H-58..H-12).contains(&y) && (115..300).contains(&x) }

pub fn size_from_slider_x(x: usize) -> u8 {
    let track_x = 125usize;
    let track_w = 145usize;
    let x = x.clamp(track_x, track_x + track_w);
    let value = ART_SIZE_MIN as usize + (x - track_x) * (ART_SIZE_MAX - ART_SIZE_MIN) as usize / track_w;
    value as u8
}

pub fn render_boot(dst: &mut [u32], disc_art: Option<&Artwork>, frame: u64) {
    dst[..W * H].fill(0xFF000000);
    let angle = frame as f32 * 0.055;
    blit_rotated(dst, disc_art.unwrap_or(&MISSING_DISC), W as i32 / 2, 254, 212, angle);
}

pub fn render_boot_details(
    dst: &mut [u32], width: usize, height: usize, view: (usize, usize, usize, usize),
    title: &str, details: &DiscDetails, synopsis: Option<&str>,
) {
    let mut canvas = NativeCanvas::overlay(dst, width, height, view);
    canvas.rect(0,0,W,H,0xFF0B1523);
    canvas.surface(18,42,248,448,12,0xFF182A3D,0xFF101D2C,0xFF2F465E);
    canvas.surface(290,146,454,150,12,0xFF1C3045,0xFF142438,0xFF354C65);
    canvas.text(24,15,"OpenCDi",0xFF99B3CC,1);
    canvas.text(690,15,"Disc info",0xFF99B3CC,1);
    canvas.line(18,34,W-36,0xFF293E53);
    if let Some(boxart) = &details.boxart { canvas.art_fit(boxart, 34, 56, 216, 255); }
    if let Some(back) = &details.back { canvas.art_fit(back, 57, 326, 170, 148); }
    if let Some(logo) = &details.logo { canvas.art_fit(logo, 320, 54, 390, 76); }
    else { canvas.centered_text(517,78,&short_title(title,30),0xFFF1F5FA,2); }
    canvas.text(307,158,"Synopsis",0xFFA9D8FA,1);
    let right=canvas.x(726).min(width);let bottom=canvas.y(284).min(height);
    if right<=canvas.x(307)||bottom<=canvas.y(181){return;}
    canvas.labels.push(NativeLabel{x:307,y:181,value:synopsis.unwrap_or("No synopsis available.").to_owned(),color:0xFFD8E5F1,scale:1,bounds:Some((right,bottom)),centered:false});
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewAction { Play, Back }

const PLAY_BUTTON: (usize, usize, usize, usize) = (312, 510, 144, 40);
const BACK_BUTTON: (usize, usize, usize, usize) = (28, 512, 142, 36);

pub fn render_preview_actions(dst: &mut [u32], width: usize, height: usize, view: (usize, usize, usize, usize)) {
    let mut canvas = NativeCanvas::overlay(dst, width, height, view);
    canvas.surface(18,504,732,50,12,0xFF1C3045,0xFF142235,0xFF3A526A);
    let (x, y, w, h) = PLAY_BUTTON;
    canvas.surface(x, y, w, h, 9, 0xFF355F80, 0xFF25445F, 0xFF6CBCF2);
    canvas.icon(17,x+31,y+10,20,0xFFBDE5FF);
    canvas.text(x+60,y+10,"Play",0xFFF1F5FA,2);
    let (x, y, w, h) = BACK_BUTTON;
    canvas.surface(x, y, w, h, 8, 0xFF253B52, 0xFF182A3D, 0xFF425F7A);
    canvas.icon(16,x+8,y+11,14,0xFFB0C4D9);
    canvas.text(x+27,y+12,"Back to Library",0xFFE6EDF5,1);
}

pub fn preview_hit(x: i32, y: i32) -> Option<PreviewAction> {
    let (x, y) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
    [(PLAY_BUTTON, PreviewAction::Play), (BACK_BUTTON, PreviewAction::Back)].into_iter()
        .find_map(|((left, top, w, h), action)| {
            ((left..left + w).contains(&x) && (top..top + h).contains(&y)).then_some(action)
        })
}

pub fn toolbar_hit(x: usize, y: usize, popup: Option<ToolbarPopup>, optical_disc: Option<char>) -> Option<ToolbarAction> {
    if popup == Some(ToolbarPopup::Settings) && toolbar_popup_hit(x, y, popup) {
        let row = y.checked_sub(SETTINGS_Y + SETTINGS_HEADER + 6)? / SETTINGS_ROW;
        return SETTINGS_ITEMS.get(row).map(|&(_, action)| action);
    }
    if y >= TOOLBAR_HEIGHT { return None; }
    dock_buttons(optical_disc).into_iter().find_map(|(action,left,width,_,_)| {
        ((left..left+width).contains(&x) && (5..37).contains(&y)).then_some(action)
    })
}

pub fn pause_menu_hit(x: usize, y: usize) -> Option<usize> {
    if !(214..554).contains(&x) { return None; }
    for row in 0..4 { let top = 234 + row * 42; if (top..top + 34).contains(&y) { return Some(row); } }
    None
}

pub const PAUSE_ACTIONS: [&str; 7] = ["Resume", "Disc Info", "Save State", "Load State", "Controller Settings", "Back to Library", "Exit OpenCDi"];
const PAUSE_LEFT:usize=350;
const PAUSE_WIDTH:usize=300;
const PAUSE_TOP:usize=128;
const PAUSE_STEP:usize=43;
const PAUSE_HEIGHT:usize=36;

pub fn pause_menu_hit_full(x: usize, y: usize, width: usize, height: usize) -> Option<usize> {
    if width==0||height==0{return None;}
    let bx=x.saturating_mul(W)/width;let by=y.saturating_mul(H)/height;
    if !(PAUSE_LEFT..PAUSE_LEFT+PAUSE_WIDTH).contains(&bx){return None;}
    (0..PAUSE_ACTIONS.len()).find(|row|{let top=PAUSE_TOP+row*PAUSE_STEP;(top..top+PAUSE_HEIGHT).contains(&by)})
}

pub fn render_pause_menu_full(dst: &mut [u32], width: usize, height: usize, title: &str, logo: Option<&Artwork>, selected: usize) {
    if width == 0 || height == 0 || dst.len() < width.saturating_mul(height) { return; }
    let src=dst[..width*height].to_vec();
    for by in (0..height).step_by(8){for bx in (0..width).step_by(8){
        let mut r=0u32;let mut g=0u32;let mut b=0u32;let mut n=0u32;
        for yy in by..(by+8).min(height){for xx in bx..(bx+8).min(width){let p=src[yy*width+xx];r+=(p>>16)&255;g+=(p>>8)&255;b+=p&255;n+=1;}}
        let avg=0xFF000000|((r/n)<<16)|((g/n)<<8)|(b/n);let tinted=blend(0xFF091321,avg,172);
        rect_full(dst,width,height,bx,by,8,8,tinted);
    }}
    let mut c=NativeCanvas::overlay(dst,width,height,(0,0,width,height));
    c.surface(90,70,588,430,15,0xFF17283C,0xFF101D2D,0xFF3A526B);
    c.text(112,88,"Game Paused",0xFFF1F5FA,2);
    c.surface(112,128,102,25,7,0xFF24435B,0xFF1D354B,0xFF416C8B);
    c.rect(122,135,2,10,0xFFB9E5FF);c.rect(127,135,2,10,0xFFB9E5FF);c.text(138,135,"Paused",0xFFB9E5FF,1);
    if let Some(logo) = logo {
        c.art_fit(logo,110,186,216,88);
    } else {
        let (first,second)=wrap_title(title,24);
        c.text(112,190,&first,0xFFF1F5FA,2);
        if !second.is_empty(){c.text(112,218,&second,0xFFF1F5FA,2);}
    }
    c.text(112,291,"Philips CD-i",0xFFA7BED6,1);
    c.text(112,362,"Press Home / Pause to resume",0xFFB5CDE2,1);
    c.text(112,388,"Point to choose with the Wiimote",0xFF91A9C1,1);
    c.text(PAUSE_LEFT,105,"ACTIONS",0xFF93ADC6,1);
    for(row,label)in PAUSE_ACTIONS.iter().enumerate(){
        let y=PAUSE_TOP+row*PAUSE_STEP;let active=row==selected;
        c.surface(PAUSE_LEFT,y,PAUSE_WIDTH,PAUSE_HEIGHT,8,
            if active{0xFF2E526E}else{0xFF23394F},if active{0xFF23435D}else{0xFF1A2C40},
            if active{0xFF68C2FA}else{0xFF3B526A});
        c.icon([17,4,14,15,1,7,5][row],PAUSE_LEFT+13,y+9,17,if active{0xFFBAE7FF}else{0xFFADC3DA});
        c.text(PAUSE_LEFT+43,y+12,label,0xFFEAF2FA,1);
    }
    c.line(112,454,538,0xFF2C425A);
    c.text(112,469,"Button I: Choose     Button II: Back",0xFFA7BED6,1);
    c.text(528,469,"Plus + Minus: Exit",0xFFA7BED6,1);
}

fn rect_full(dst: &mut [u32], width: usize, height: usize, x: usize, y: usize, w: usize, h: usize, color: u32) {
    for yy in y.min(height)..y.saturating_add(h).min(height) {
        let start = yy * width + x.min(width);
        let end = yy * width + x.saturating_add(w).min(width);
        if start < end && end <= dst.len() { dst[start..end].fill(color); }
    }
}

fn text_full(dst: &mut [u32], width: usize, height: usize, x: usize, y: usize, value: &str, color: u32, scale: usize) {
    let scale = scale.max(1);
    let mut px = x;
    for ch in value.chars().take(64) {
        if ch == ' ' { px += 4 * scale; continue; }
        let rows = glyph(ch);
        for (gy, row) in rows.iter().enumerate() {
            for gx in 0..5 {
                if row & (1 << (4 - gx)) != 0 { rect_full(dst, width, height, px + gx * scale, y + gy * scale, scale, scale, color); }
            }
        }
        px += 6 * scale;
    }
}

pub fn render_pause_menu(dst: &mut [u32], title: &str, selected: usize) {
    for pixel in dst.iter_mut() { *pixel = blend(0xFF000000, *pixel, 150); }
    let x = 204usize; let y = 142usize; let w = 360usize; let h = 298usize;
    rect(dst, x + 5, y + 5, w, h, 0x99000000);
    rect(dst, x, y, w, h, 0xFF111B2A);
    rect(dst, x, y, w, 2, 0xFF6DB9F7);
    text(dst, x + 22, y + 20, "GAME PAUSED", 0xFF8CC9FF, 1);
    text(dst, x + 22, y + 42, &title.chars().take(40).collect::<String>(), 0xFFF4F6F9, 2);
    text(dst, x + 22, y + 66, "GUIDE HOME TO RESUME", 0xFF9EADBE, 1);
    for (row, label) in ["RESUME", "SAVE STATE", "LOAD STATE", "STOP AND RETURN TO MENU"].iter().enumerate() {
        let ry = y + 91 + row * 42;
        if row == selected { rect(dst, x + 14, ry, w - 28, 34, 0xFF294765); rect(dst, x + 14, ry, 3, 34, 0xFF70C0FF); }
        text(dst, x + 30, ry + 12, label, if row == selected { 0xFFFFFFFF } else { 0xFFD0D8E2 }, 1);
    }
    rect(dst, x + 18, y + h - 37, w - 36, 1, 0xFF344458);
    text(dst, x + 22, y + h - 22, "DPAD OR STICK: SELECT    A / ENTER: CHOOSE", 0xFF9EADBE, 1);
}

fn blit_rotated(dst: &mut [u32], art: &Artwork, cx: i32, cy: i32, size: i32, angle: f32) {
    blit_rotated_into(dst, W, H, art, cx, cy, size, angle);
}

fn blit_rotated_into(dst: &mut [u32], width: usize, height: usize, art: &Artwork, cx: i32, cy: i32, size: i32, angle: f32) {
    let (sin, cos) = angle.sin_cos(); let radius = size / 2;
    for oy in -radius..radius { for ox in -radius..radius {
        let u = ox as f32 * cos / size as f32 + oy as f32 * sin / size as f32 + 0.5;
        let v = -ox as f32 * sin / size as f32 + oy as f32 * cos / size as f32 + 0.5;
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) { continue; }
        let x = cx + ox; let y = cy + oy;
        if x < 0 || y < 0 || x as usize >= width || y as usize >= height { continue; }
        let src = sample_bilinear(art, u * art.w as f32 - 0.5, v * art.h as f32 - 0.5); let a = src >> 24;
        let idx = y as usize * width + x as usize; dst[idx] = blend(src, dst[idx], a);
    }}
}

pub(crate) fn blit_fit(dst: &mut [u32], art: &Artwork, x: usize, y: usize, w: usize, h: usize) {
    let scale = (w as f32 / art.w as f32).min(h as f32 / art.h as f32);
    let rw = (art.w as f32 * scale).max(1.0) as usize; let rh = (art.h as f32 * scale).max(1.0) as usize;
    let ox = x + (w - rw) / 2; let oy = y + (h - rh) / 2;
    for dy in 0..rh { for dx in 0..rw {
        let sx = (dx as f32 + 0.5) * art.w as f32 / rw as f32 - 0.5;
        let sy = (dy as f32 + 0.5) * art.h as f32 / rh as f32 - 0.5;
        let src = sample_bilinear(art, sx, sy);
        let a = (src >> 24) & 255; let di = (oy + dy) * W + ox + dx;
        if di < dst.len() { dst[di] = blend(src, dst[di], a); }
    }}
}

pub(crate) fn sample_bilinear(art: &Artwork, x: f32, y: f32) -> u32 {
    let x = x.clamp(0.0, art.w.saturating_sub(1) as f32);
    let y = y.clamp(0.0, art.h.saturating_sub(1) as f32);
    let x0 = x.floor() as usize; let y0 = y.floor() as usize;
    let x1 = (x0 + 1).min(art.w - 1); let y1 = (y0 + 1).min(art.h - 1);
    let fx = x - x0 as f32; let fy = y - y0 as f32;
    let pixels = [art.pixels[y0 * art.w + x0], art.pixels[y0 * art.w + x1], art.pixels[y1 * art.w + x0], art.pixels[y1 * art.w + x1]];
    let weights = [(1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy];
    let mut alpha = 0.0; let mut red = 0.0; let mut green = 0.0; let mut blue = 0.0;
    for i in 0..4 {
        let a = ((pixels[i] >> 24) & 255) as f32 / 255.0;
        alpha += a * weights[i];
        red += ((pixels[i] >> 16) & 255) as f32 * a * weights[i];
        green += ((pixels[i] >> 8) & 255) as f32 * a * weights[i];
        blue += (pixels[i] & 255) as f32 * a * weights[i];
    }
    if alpha <= 0.0001 { return 0; }
    let a = (alpha * 255.0).round() as u32;
    let r = (red / alpha).round().clamp(0.0, 255.0) as u32;
    let g = (green / alpha).round().clamp(0.0, 255.0) as u32;
    let b = (blue / alpha).round().clamp(0.0, 255.0) as u32;
    (a << 24) | (r << 16) | (g << 8) | b
}

pub(crate) fn blend(src: u32, dst: u32, a: u32) -> u32 {
    let inv = 255 - a;
    let r = (((src >> 16) & 255) * a + ((dst >> 16) & 255) * inv) / 255;
    let g = (((src >> 8) & 255) * a + ((dst >> 8) & 255) * inv) / 255;
    let b = ((src & 255) * a + (dst & 255) * inv) / 255;
    0xFF00_0000 | (r << 16) | (g << 8) | b
}

pub(crate) fn rect(dst: &mut [u32], x: usize, y: usize, w: usize, h: usize, color: u32) {
    for yy in y..(y + h).min(H) { let start = yy * W + x.min(W); let end = (yy * W + (x + w).min(W)).min(dst.len()); if start < end { dst[start..end].fill(color); } }
}

fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        'A'=>[14,17,17,31,17,17,17],'B'=>[30,17,17,30,17,17,30],'C'=>[14,17,16,16,16,17,14],
        'D'=>[30,17,17,17,17,17,30],'E'=>[31,16,16,30,16,16,31],'F'=>[31,16,16,30,16,16,16],
        'G'=>[14,17,16,23,17,17,15],'H'=>[17,17,17,31,17,17,17],'I'=>[14,4,4,4,4,4,14],
        'J'=>[7,2,2,2,18,18,12],'K'=>[17,18,20,24,20,18,17],'L'=>[16,16,16,16,16,16,31],
        'M'=>[17,27,21,21,17,17,17],'N'=>[17,25,21,19,17,17,17],'O'=>[14,17,17,17,17,17,14],
        'P'=>[30,17,17,30,16,16,16],'Q'=>[14,17,17,17,21,18,13],'R'=>[30,17,17,30,20,18,17],
        'S'=>[15,16,16,14,1,1,30],'T'=>[31,4,4,4,4,4,4],'U'=>[17,17,17,17,17,17,14],
        'V'=>[17,17,17,17,17,10,4],'W'=>[17,17,17,21,21,21,10],'X'=>[17,17,10,4,10,17,17],
        'Y'=>[17,17,10,4,4,4,4],'Z'=>[31,1,2,4,8,16,31],
        '0'=>[14,17,19,21,25,17,14],'1'=>[4,12,4,4,4,4,14],'2'=>[14,17,1,2,4,8,31],
        '3'=>[30,1,1,14,1,1,30],'4'=>[2,6,10,18,31,2,2],'5'=>[31,16,16,30,1,1,30],
        '6'=>[14,16,16,30,17,17,14],'7'=>[31,1,2,4,8,8,8],'8'=>[14,17,17,14,17,17,14],
        '9'=>[14,17,17,15,1,1,14], ':' => [0,4,4,0,4,4,0], '-' => [0,0,0,31,0,0,0],
        '.' => [0,0,0,0,0,12,12], '/' => [1,2,2,4,8,8,16], '+' => [0,4,4,31,4,4,0],
        _=>[0;7]
    }
}

pub(crate) fn text(dst: &mut [u32], x: usize, y: usize, s: &str, color: u32, scale: usize) {
    let mut px = x;
    for ch in s.chars().take(54) { if ch == ' ' { px += 4 * scale; continue; }
        let g = glyph(ch);
        for (gy,row) in g.iter().enumerate() { for gx in 0..5 { if row & (1 << (4-gx)) != 0 { rect(dst, px + gx*scale, y + gy*scale, scale, scale, color); } } }
        px += 6 * scale;
    }
}

pub fn slot_at(x: usize, y: usize, art_size: u8) -> Option<usize> {
    let l = layout(art_size);
    if y < l.top { return None; }
    let dy = y - l.top;
    let row = dy / (l.cell_h + l.gap);
    if row >= l.rows || dy % (l.cell_h + l.gap) >= l.cell_h { return None; }
    let dx = x.checked_sub(l.left)?;
    let col = dx / (l.cell_w + l.gap);
    if col >= l.columns || dx % (l.cell_w + l.gap) >= l.cell_w { None } else { Some(row * l.columns + col) }
}

#[cfg(test)]
mod library_tests {
    use super::*;

    #[test]
    fn pause_actions_and_pointer_targets_match_at_different_window_sizes() {
        assert_eq!(PAUSE_ACTIONS,["Resume","Disc Info","Save State","Load State","Controller Settings","Back to Library","Exit OpenCDi"]);
        for (width,height) in [(768,560),(1920,1080),(1024,768)] {
            for row in 0..PAUSE_ACTIONS.len() {
                let x=(PAUSE_LEFT+PAUSE_WIDTH/2)*width/W;
                let y=(PAUSE_TOP+row*PAUSE_STEP+PAUSE_HEIGHT/2)*height/H;
                assert_eq!(pause_menu_hit_full(x,y,width,height),Some(row));
                let gap=(PAUSE_TOP+row*PAUSE_STEP+PAUSE_HEIGHT+3)*height/H;
                assert_eq!(pause_menu_hit_full(x,gap,width,height),None);
            }
            assert_eq!(pause_menu_hit_full(0,0,width,height),None);
        }
    }

    #[test]
    fn artwork_names_migrate_without_redownloads_or_overwriting_existing_files() {
        let root=std::env::temp_dir().join(format!("opencdi-art-names-{}",std::process::id()));
        std::fs::create_dir_all(root.join("Boxart")).unwrap();
        std::fs::create_dir_all(root.join("CD")).unwrap();
        let disc=PathBuf::from("Mad Dog McCree (USA, Europe).chd");
        let old_box=root.join("Boxart/Mad Dog McCree (USA, Europe)--generated.png");
        let new_box=root.join("Boxart/Mad Dog McCree (USA, Europe).png");
        let old_cd=root.join("CD/Mad Dog McCree (USA, Europe)-disc.png");
        let new_cd=root.join("CD/Mad Dog McCree (USA, Europe).png");
        std::fs::write(&old_box,b"existing box art").unwrap();
        std::fs::write(&old_cd,b"existing disc art").unwrap();
        migrate_artwork_names(std::slice::from_ref(&disc),&root);
        assert_eq!(std::fs::read(&new_box).unwrap(),b"existing box art");
        assert_eq!(std::fs::read(&new_cd).unwrap(),b"existing disc art");
        assert_eq!(managed_art_path(&disc,&root,crate::launchbox::ArtworkKind::Boxart),Some(new_box.clone()));
        std::fs::write(&old_cd,b"older duplicate").unwrap();
        migrate_artwork_names(&[disc],&root);
        assert_eq!(std::fs::read(&new_cd).unwrap(),b"existing disc art");
        assert_eq!(std::fs::read(&old_cd).unwrap(),b"older duplicate");
        for path in [old_cd,new_cd,new_box] {std::fs::remove_file(path).unwrap();}
        std::fs::remove_dir(root.join("Boxart")).unwrap();
        std::fs::remove_dir(root.join("CD")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn cached_boot_scene_refreshes_changed_artwork_controls_and_window_size() {
        let mut scene = BootScene::default();
        let art = |color| Artwork { w: 2, h: 2, pixels: vec![color; 4].into() };
        let mut details = DiscDetails { boxart: Some(art(0xFF00FF00)), ..DiscDetails::default() };
        let mut output = vec![0; W * H];
        scene.render(&mut output, W, H, (0,0,W,H), "Test", &details, None, true, None, 0);
        assert_eq!(output[100 * W + 100], 0xFF00FF00);
        assert_ne!(output[530 * W + 330], 0xFF000000);
        scene.render(&mut output, W, H, (0,0,W,H), "Test", &details, None, false, None, 20);
        assert_eq!(output[100 * W + 100], 0xFF00FF00);
        assert_eq!(output[530 * W + 330], 0xFF0B1523);
        details.boxart = Some(art(0xFFFF0000));
        scene.render(&mut output, W, H, (0,0,W,H), "Test", &details, None, false, None, 40);
        assert_eq!(output[100 * W + 100], 0xFFFF0000);
        output.resize(1280 * 720, 0);
        scene.render(&mut output, 1280, 720, (0,0,1280,720), "Changed", &details, Some("Synopsis"), false, None, 60);
        assert_eq!(scene.pixels.len(), output.len());
        assert_eq!(scene.title, "Changed");
    }

    #[test]
    fn category_search_and_collection_controls_leave_the_grid_and_slider_available() {
        let items = ["Flashback (Europe)", "Space Safari (UK)", "Uncatalogued disc"].map(|title| {
            Item { path: format!("{title}.chd").into(), title: title.into(), cover: None, disc: None, logo: None }
        });
        let mut search = Search::default();
        search.collection.refresh(&items);
        assert_eq!(search.len(&items), 3);
        search.collection.category = Category::Kids;
        search.rebuild(&items);
        assert_eq!(search.len(&items), 1);
        assert_eq!(search.get(&items, 0).unwrap().title, "Space Safari (UK)");
        assert_eq!(search.collection.owned_count(), 1);
        let missing = search.collection.missing_count();
        search.insert("Flashback");
        search.rebuild(&items);
        assert_eq!(search.len(&items), 0);
        assert_eq!(search.collection.missing_count(), missing);
        assert_eq!(category_hit(14 + 6 * TAB_WIDTH, TAB_TOP), Some(Category::Homebrew));
        assert!(TAB_TOP as usize + (TAB_HEIGHT as usize) < layout(ART_SIZE_MIN).top);
        assert!(size_slider_hit(160, H - 35));
        assert!(!collection_count_hit(160, (H - 35) as i32));
        assert!(collection_count_hit(615, (H - 79) as i32));
        assert!(!size_slider_hit(615, H - 79));
    }

    #[test]
    fn search_filters_titles_and_filenames_without_losing_source_items() {
        let items = [
            Item { path: "Shadoan (Europe).chd".into(), title: "Kingdom II".into(), cover: None, disc: None, logo: None },
            Item { path: "Dragons Lair (EUR).chd".into(), title: "Dragon’s Lair".into(), cover: None, disc: None, logo: None },
            Item { path: "Flashback (Europe).chd".into(), title: "Flashback".into(), cover: None, disc: None, logo: None },
        ];
        let mut search = Search::default();
        assert!(search.insert("SHADOAN kingdom"));
        search.rebuild(&items);
        assert_eq!(search.len(&items), 1);
        assert_eq!(search.get(&items, 0).unwrap().path, items[0].path);
        search.clear();
        assert!(search.insert("dragon's-lair"));
        search.rebuild(&items);
        assert_eq!(search.position(&items, &items[1].path), Some(0));
        search.clear();
        assert!(search.insert("does not exist"));
        search.rebuild(&items);
        assert!(search.get(&items, 0).is_none());
        search.clear();
        search.rebuild(&items);
        assert_eq!(search.len(&items), 3);
    }

    #[test]
    fn settings_menu_covers_deferred_library_text() {
        let render = |underlying_label: &str| {
            let mut pixels = vec![0; W * H];
            {
                let mut canvas = NativeCanvas::new(&mut pixels, W, H, (0, 0, W, H));
                canvas.text(SETTINGS_X + 25, SETTINGS_Y + 35, underlying_label, 0xFFFFFFFF, 1);
                render_toolbar_native(&mut canvas, DisplayMode::Windowed, Some(ToolbarPopup::Settings), None, None, Some(("Philips CD-i 490","cdi490a.rom")));
            }
            pixels
        };
        assert!(render("Library text") == render(""));
    }

    #[test]
    fn search_editing_handles_multibyte_text() {
        let mut search = Search::default();
        assert!(search.insert("Café"));
        search.move_caret(false);
        assert!(search.delete(false));
        assert_eq!(search.query, "Caf");
        search.select_all = true;
        assert!(search.insert("CD-i"));
        assert_eq!(search.query, "CD-i");
        assert_eq!(search_hit(689, 58), Some(true));
        assert_eq!(search_hit(500, 58), Some(false));
        assert_eq!(search_hit(500, 90), None);
    }

    #[test]
    fn smallest_grid_exposes_forty_clickable_slots() {
        let l = layout(ART_SIZE_MIN);
        assert_eq!((l.columns, l.rows), (10, 4));
        for slot in 0..40 {
            let x = l.left + (slot % l.columns) * (l.cell_w + l.gap);
            let y = l.top + (slot / l.columns) * (l.cell_h + l.gap);
            assert_eq!(slot_at(x, y, ART_SIZE_MIN), Some(slot));
        }
        assert!(l.top + l.rows * l.cell_h + (l.rows - 1) * l.gap < H - 64);
    }

    #[test]
    fn play_disc_toolbar_action_requires_detected_media() {
        assert_eq!(toolbar_hit(45, 16, None, None), Some(ToolbarAction::OpenDisc));
        assert_eq!(toolbar_hit(45, 16, None, Some('F')), Some(ToolbarAction::PlayDisc('F')));
        assert_eq!(toolbar_hit(120, 16, None, Some('F')), Some(ToolbarAction::OpenDisc));
        assert_eq!(toolbar_hit(325,16,None,None),Some(ToolbarAction::Start220));
        assert_eq!(toolbar_hit(410,16,None,Some('F')),Some(ToolbarAction::Start490));
    }

    #[test]
    fn toolbar_uses_new_positions_and_paths_has_four_actions() {
        assert!(toolbar_hit(590, 16, None, None) == Some(ToolbarAction::Settings));
        assert!(toolbar_hit(660, 16, None, None) == Some(ToolbarAction::About));
        for (row, (_, action)) in SETTINGS_ITEMS.into_iter().enumerate() {
            assert!(toolbar_hit(SETTINGS_X+25, SETTINGS_Y + SETTINGS_HEADER + 9 + row * SETTINGS_ROW, Some(ToolbarPopup::Settings), None) == Some(action));
        }
        assert_eq!(toolbar_hit(SETTINGS_X+25,SETTINGS_Y+2,Some(ToolbarPopup::Settings),None),None);
        assert_eq!(preview_hit(384, 530), Some(PreviewAction::Play));
        assert_eq!(preview_hit(64, 540), Some(PreviewAction::Back));
        assert_eq!(preview_hit(384, 500), None);
    }

    #[test]
    fn rescan_detects_discs_added_and_removed_in_subfolders() {
        let root = std::env::temp_dir().join(format!("opencdi-library-scan-{}", std::process::id()));
        let discs = root.join("Discs");
        let path = discs.join("Example.chd");
        std::fs::create_dir_all(&discs).unwrap();
        assert!(scan_paths(std::slice::from_ref(&root)).is_empty());
        std::fs::write(&path, []).unwrap();
        assert_eq!(scan_paths(std::slice::from_ref(&root)), [path.clone()]);
        std::fs::remove_file(path).unwrap();
        assert!(scan_paths(std::slice::from_ref(&root)).is_empty());
        std::fs::remove_dir(discs).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn case_overlay_keeps_cover_pixels_unchanged_and_draws_the_frame_separately() {
        let cover = Artwork { w: 2, h: 2, pixels: vec![0xff336699; 4].into() };
        let original = cover.pixels.clone();
        let mut output = vec![0xff101010; CASE_WIDTH * CASE_HEIGHT];
        blit_cover_fit(&mut output, CASE_WIDTH, CASE_HEIGHT, &cover,
            0, 0, CASE_WIDTH, CASE_HEIGHT);
        assert_eq!(cover.pixels.as_ref(), original.as_ref());
        assert_eq!(output[306 * CASE_WIDTH + 362], 0xff336699);
        assert_ne!(output[306 * CASE_WIDTH + 30], 0xff336699);
        assert_eq!(COVER_FRAME.pixels[306 * CASE_WIDTH + 362], 0);
    }

    #[test]
    fn boot_synopsis_wraps_to_the_right_in_windowed_and_wide_views() {
        let summary = "Conrad must travel across four planets and return to Earth to stop the aliens. ".repeat(8);
        for (width, height, view) in [(W, H, (0, 0, W, H)), (1920, 1080, (219, 0, 1481, 1080))] {
            let mut pixels = vec![0; width * height];
            render_boot_details(&mut pixels, width, height, view, "", &DiscDetails::default(), Some(&summary));
            let mut without=vec![0;width*height];
            render_boot_details(&mut without,width,height,view,"",&DiscDetails::default(),Some(""));
            let left=view.0+307*view.2/W;let right=view.0+726*view.2/W;
            let top=view.1+181*view.3/H;let bottom=view.1+284*view.3/H;
            let changed:Vec<_>=pixels.iter().zip(&without).enumerate().filter(|(_, (a,b))|a!=b).map(|(i,_)|(i%width,i/width)).collect();
            assert!(!changed.is_empty());
            assert!(changed.iter().all(|&(x,y)|x>=left&&x<right&&y>=top&&y<bottom));

        }
    }

    #[test]
    fn missing_cover_keeps_its_title_without_a_box() {
        let mut pixels = vec![0; W * H];
        let item = Item { path: "Example.chd".into(), title: "Example".into(), cover: None, disc: None, logo: None };
        render_native(&mut pixels, W, H, (0, 0, W, H), &[item], 0, ART_SIZE_MIN, true, DisplayMode::Windowed, None, None, &Search::default(), None);
        let l = layout(ART_SIZE_MIN);
        assert_eq!(pixels[(l.top + 5) * W + l.left + 1], pixels[(l.top + 5) * W + l.left - 1]);
        assert_eq!(MISSING_COVER.w, 680);
        assert_eq!(MISSING_DISC.w, 635);
        assert_eq!(MISSING_DISC.pixels[0] >> 24, 0);
        assert!(pixels.iter().any(|&pixel| pixel & 0xFFFFFF == 0xFFFFFF));
        render_boot(&mut pixels, None, 0);
        assert_eq!(pixels[254 * W + W / 2], 0xFF000000);
        assert_ne!(pixels[254 * W + W / 2 + 70], 0xFF000000);
        assert_eq!(wrap_title("Brain Dead 13", 9), ("Brain".into(), "Dead 13".into()));
    }
}
