use std::collections::HashSet;
use std::path::{Path, PathBuf};
use opencdi::frontend::{ART_SIZE_MAX, ART_SIZE_MIN};
use opencdi::frontend::DisplayMode;

pub const W: usize = 768;
pub const H: usize = 560;
const MAX_ITEMS: usize = 512;
pub const TOOLBAR_HEIGHT: usize = 42;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolbarPopup { Paths, Display }

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToolbarAction { OpenDisc, Controller, Paths, Refresh, Display, Exit, AddDiscFolder, SetCovers, SetCd, SetBezels, DownloadArtwork, Borderless, Decorated, Windowed }
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
    let scale = art_size.clamp(ART_SIZE_MIN, ART_SIZE_MAX) as f32 / 100.0;
    let gap = 10;
    let left = 14;
    let available_w = W - left * 2;
    let desired_w = (128.0 * scale).round() as usize;
    let columns = ((available_w + gap) / (desired_w + gap)).clamp(3, 8);
    let cell_w = (available_w - gap * (columns - 1)) / columns;
    let top = 108;
    let bottom = 492;
    let desired_h = (120.0 * scale + 24.0).round() as usize;
    let rows = ((bottom - top + gap) / (desired_h + gap)).clamp(1, 4);
    let cell_h = ((bottom - top - gap * (rows - 1)) / rows).min(desired_h);
    LibraryLayout { columns, rows, cell_w, cell_h, gap, left, top }
}

#[derive(Clone)]
pub struct Artwork { pub w: usize, pub h: usize, pub pixels: Vec<u32> }

/// Draws menu geometry on the window-sized surface while keeping layout coordinates
/// in the emulator UI's reference space. This avoids enlarging a low-resolution menu.
pub struct NativeCanvas<'a> {
    dst: &'a mut [u32], pub width: usize, pub height: usize,
    view: (usize, usize, usize, usize),
    labels: Vec<NativeLabel>,
}

struct NativeLabel { x: usize, y: usize, value: String, color: u32, scale: usize }

impl<'a> NativeCanvas<'a> {
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
        self.labels.push(NativeLabel { x, y, value: value.chars().take(96).collect(), color, scale });
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

#[cfg(windows)]
impl Drop for NativeCanvas<'_> {
    fn drop(&mut self) { draw_labels_gdi(self); }
}

#[cfg(not(windows))]
impl Drop for NativeCanvas<'_> {
    fn drop(&mut self) {
        let labels=std::mem::take(&mut self.labels);
        for label in labels { self.legacy_text(label.x,label.y,&label.value,label.color,label.scale); }
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
    fn TextOutW(dc:*mut std::ffi::c_void,x:i32,y:i32,text:*const u16,count:i32)->i32;
    fn DeleteObject(obj:*mut std::ffi::c_void)->i32;
    fn DeleteDC(dc:*mut std::ffi::c_void)->i32;
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
            TextOutW(dc,x,y,text.as_ptr(),text.len() as i32);SelectObject(dc,old_font);
        }
        std::ptr::copy_nonoverlapping(bits as *const u32,c.dst.as_mut_ptr(),c.width*c.height);
        for (_,font) in fonts{DeleteObject(font);}SelectObject(dc,old_bitmap);DeleteObject(bitmap);DeleteDC(dc);
    }
}

#[derive(Clone)]
pub struct Item { pub path: PathBuf, pub title: String, pub cover: Option<Artwork>, pub disc: Option<Artwork> }

pub fn scan(roots: &[PathBuf], covers_root: &Path, cd_root: &Path, fallback_covers: &Path, fallback_cd: &Path) -> Vec<Item> {
    let mut paths = Vec::new();
    let mut seen = HashSet::new();
    for root in roots { scan_dir(root, 0, &mut seen, &mut paths); }
    let cue_stems: HashSet<String> = paths.iter().filter(|p| ext(p) == "cue")
        .filter_map(|p| p.file_stem()).map(|s| s.to_string_lossy().to_lowercase()).collect();
    paths.retain(|p| !(ext(p) == "bin" && p.file_stem().map(|s| cue_stems.contains(&s.to_string_lossy().to_lowercase())).unwrap_or(false)));
    paths.sort_by_key(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default());
    paths.into_iter().take(MAX_ITEMS).map(|path| {
        let title = path.file_stem().map(|s| s.to_string_lossy().replace('_', " ")).unwrap_or_else(|| "CD-i Disc".into());
        let cover = load_cover_artwork(&path, covers_root).or_else(|| load_cover_artwork(&path, fallback_covers));
        let disc = load_disc_artwork(&path, cd_root).or_else(|| load_disc_artwork(&path, fallback_cd));
        Item { path, title, cover, disc }
    }).collect()
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

fn sibling_art(disc: &Path, cover: bool) -> Option<PathBuf> {
    let parent = disc.parent().unwrap_or_else(|| Path::new("."));
    let stem = disc.file_stem()?.to_string_lossy();
    let kind = if cover { "cover" } else { "disc" };
    let mut candidates = Vec::new();
    for (suffix, dir) in if cover { vec![("", "covers"), ("-cover", "covers"), ("_cover", "covers"), (".cover", "artwork")] }
                          else { vec![("-disc", "discs"), ("_disc", "discs"), (".disc", "artwork"), ("-disk", "discs")] } {
        for ext in ["png", "jpg", "jpeg"] {
            candidates.push(parent.join(dir).join(format!("{stem}{suffix}.{ext}")));
            candidates.push(parent.join("artwork").join(format!("{stem}{suffix}.{ext}")));
        }
    }
    for ext in ["png", "jpg", "jpeg"] {
        candidates.push(parent.join(format!("{stem}-{kind}.{ext}")));
        if cover { candidates.push(parent.join(format!("{stem}.{ext}"))); }
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn art_in_root(disc: &Path, root: &Path, cover: bool) -> Option<PathBuf> {
    let stem = disc.file_stem()?.to_string_lossy();
    let names = if cover { vec![format!("{stem}"), format!("{stem}-cover"), format!("{stem}_cover")] }
                else { vec![format!("{stem}"), format!("{stem}-disc"), format!("{stem}-cd"), format!("{stem}_disc")] };
    for name in names {
        for ext in ["png", "jpg", "jpeg"] {
            let candidate = root.join(format!("{name}.{ext}"));
            if candidate.is_file() { return Some(candidate); }
        }
    }
    None
}

pub fn load_cover_artwork(disc: &Path, root: &Path) -> Option<Artwork> {
    art_in_root(disc, root, true).or_else(|| sibling_art(disc, true)).and_then(|p| load_png(&p))
}

pub fn load_disc_artwork(disc: &Path, root: &Path) -> Option<Artwork> {
    art_in_root(disc, root, false).or_else(|| sibling_art(disc, false)).and_then(|p| load_png(&p))
}

fn load_png(path: &Path) -> Option<Artwork> {
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    decode_png(file).or_else(|| decode_wic(path))
}

#[cfg(windows)]
fn decode_wic(path: &Path) -> Option<Artwork> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::GENERIC_READ;
    use windows::Win32::Graphics::Imaging::{CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand};
    use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize};
    struct ComInit;
    impl Drop for ComInit { fn drop(&mut self) { unsafe { CoUninitialize(); } } }
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() { return None; }
        let _com = ComInit;
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
        text(dst, 135, 250, "ADD CHD ISO OR CUE BIN FILES BESIDE THE APP", 0xFFE1E5ED, 1);
        text(dst, 200, 280, "OR PLACE THEM IN A GAMES, DISCS OR ROMS FOLDER", 0xFFE1E5ED, 1);
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
            if let Some(cover) = &items[i].cover { blit_fit(dst, cover, art_x, art_y, art_w, art_h); }
            else { fallback_cover(dst, art_x + art_w / 6, art_y, art_w * 2 / 3, art_h); }
            let title: String = items[i].title.chars().take(24).collect();
            text(dst, x + 4, y + layout.cell_h - 30, &title, if active { 0xFFFFD98A } else { 0xFFE6EAF0 }, 1);
            let format = ext(&items[i].path).to_ascii_uppercase();
            text(dst, x + 4, y + layout.cell_h - 15, &format, 0xFF93A7BC, 1);
            if active { rect(dst, x + 4, y + layout.cell_h - 3, layout.cell_w.saturating_sub(8), 2, 0xFFFFC45B); }
        }
        let pages = (items.len() + per_page - 1) / per_page;
        text(dst, 23, 512, "ARROWS OR GAMEPAD D PAD / STICK: MOVE     ENTER / A: PLAY", 0xFFD5DCE7, 1);
        text(dst, 23, 530, "MOUSE: SELECT / DOUBLE CLICK: PLAY     WHEEL: SCROLL     ESC: QUIT", 0xFFD5DCE7, 1);
        text(dst, 717, 544, &format!("{}/{}", selected / per_page + 1, pages), 0xFFFFD98A, 1);
    }
}

pub fn render_native(dst: &mut [u32], width: usize, height: usize, view: (usize,usize,usize,usize), items: &[Item], selected: usize, art_size: u8, toolbar_visible: bool, mode: DisplayMode, popup: Option<ToolbarPopup>) {
    let mut c=NativeCanvas::new(dst,width,height,view);
    for y in 0..H { let t=y as u32*10/H as u32; let color=0xFF000000|((10+t)<<16)|((20+t)<<8)|(34+t*2); c.rect(0,y,W,1,color); }
    c.round_rect(12,50,W-24,45,10,0xFF101D2C);
    c.text(25,64,"CD-I LIBRARY",0xFFF1F5FA,2);
    c.round_rect(455,58,255,29,8,0xFF1A293B); c.ring(472,72,5,0xFF91A9C2); c.line(476,76,6,0xFF91A9C2); c.text(486,67,"Search library...",0xFF8295AA,1);
    c.rect(0,H-64,W,64,0xFF0D1725); c.rect(0,H-64,W,1,0xFF26374A);
    c.text(18,H-48,"ARTWORK SIZE",0xFFB7C5D4,1);
    let track_x=125; let track_w=145; let track_y=H-35;
    c.round_rect(track_x,track_y,track_w,4,2,0xFF33465A);
    let value=art_size.clamp(ART_SIZE_MIN,ART_SIZE_MAX);
    let filled=(value-ART_SIZE_MIN) as usize*track_w/(ART_SIZE_MAX-ART_SIZE_MIN) as usize;
    c.round_rect(track_x,track_y,filled,4,2,0xFF57AEF5);
    c.round_rect(track_x+filled.saturating_sub(5),track_y.saturating_sub(5),12,14,6,0xFFF4F7FB);
    c.text(282,H-42,&format!("{}%",value),0xFFD5DFEA,1);
    c.round_rect(326,H-51,26,28,6,0xFF243B55); c.rect(333,H-44,5,5,0xFF72BBF7);c.rect(340,H-44,5,5,0xFF72BBF7);c.rect(333,H-37,5,5,0xFF72BBF7);c.rect(340,H-37,5,5,0xFF72BBF7);
    c.round_rect(359,H-51,26,28,6,0xFF172638); c.rect(366,H-44,12,2,0xFF9FB1C5);c.rect(366,H-39,12,2,0xFF9FB1C5);c.rect(366,H-34,12,2,0xFF9FB1C5);
    c.text(660,H-42,&format!("{} ITEMS",items.len()),0xFF9AAFC4,1);
    let l=layout(value); let per_page=l.visible_count();
    if items.is_empty() {
        c.text(205,210,"NO DISC IMAGES FOUND",0xFFF1F5FA,2);
        c.text(135,250,"ADD A DISC FOLDER FROM PATHS TO BEGIN",0xFF9AAFC4,1);
        c.text(180,290,"OR CHOOSE OPEN DISC",0xFF78BDF5,1);
    } else {
        let page_start=(selected/per_page)*per_page;
        for slot in 0..per_page {
            let i=page_start+slot; if i>=items.len(){break;}
            let col=slot%l.columns; let row=slot/l.columns;
            let x=l.left+col*(l.cell_w+l.gap); let y=l.top+row*(l.cell_h+l.gap);
            let art_x=x+4; let art_y=y+2; let art_w=l.cell_w.saturating_sub(8); let art_h=l.cell_h.saturating_sub(34);
            c.round_rect(art_x-2,art_y-2,art_w+4,art_h+4,7,0xFF25364A);
            if let Some(cover)=&items[i].cover { c.art_fit(cover,art_x,art_y,art_w,art_h); }
            else { native_missing_cover(&mut c,art_x+art_w/6,art_y,art_w*2/3,art_h); }
            let title_cap=l.cell_w.saturating_sub(12)/6;
            let title:String=items[i].title.chars().take(title_cap.max(1)*2).collect();
            let active=i==selected;
            let (one,two)=wrap_title(&title,title_cap.max(1));
            let color=if active{0xFF80C4FF}else{0xFFE8EEF5};
            c.text(x+4,y+l.cell_h-26,&one,color,1);
            if !two.is_empty(){c.text(x+4,y+l.cell_h-13,&two,if active{0xFF80C4FF}else{0xFFB5C3D1},1);}
            if active { c.round_rect(x+3,y+l.cell_h-4,l.cell_w.saturating_sub(6),2,1,0xFF56B1F7); }
        }
        let pages=(items.len()+per_page-1)/per_page;
        c.text(18,H-18,"MOUSE: SELECT  ·  DOUBLE CLICK: PLAY    D-PAD / STICK: MOVE",0xFF778DA5,1);
        c.text(686,H-18,&format!("{}/{}",selected/per_page+1,pages),0xFFB7C5D4,1);
    }
    // Draw popovers over the grid so they remain legible when they extend below the toolbar.
    if toolbar_visible { render_toolbar_native(&mut c,mode,popup); }
}

fn wrap_title(title:&str,width:usize)->(String,String){
    if title.chars().count()<=width{return(title.to_string(),String::new());}
    let mut one=String::new();let mut two=String::new();
    for word in title.split_whitespace(){
        if one.is_empty(){one=word.to_string();continue;}
        if one.chars().count()+1+word.chars().count()<=width{one.push(' ');one.push_str(word);}
        else if two.is_empty(){two=word.to_string();}
        else if two.chars().count()+1+word.chars().count()<=width{two.push(' ');two.push_str(word);}
    }
    (one.chars().take(width).collect(),two.chars().take(width).collect())
}

fn native_missing_cover(c:&mut NativeCanvas<'_>,x:usize,y:usize,w:usize,h:usize){
    c.rect(x,y,w,h,0xFF263343); c.rect(x+5,y+5,w.saturating_sub(10),h.saturating_sub(10),0xFF1D2A39);
    let r=(w.min(h)*29/100).max(8); let cx=x+w/2; let cy=y+h*43/100;
    for dy in -(r as isize)..=(r as isize) {
        let dx=((r*r) as f64-(dy*dy) as f64).max(0.0).sqrt() as isize;
        let mut sx=-(dx as isize);
        while sx<=dx {
            let stripe=(sx+dy).rem_euclid(14)<5;
            let mut end=sx+1;
            while end<=dx && ((end+dy).rem_euclid(14)<5)==stripe {end+=1;}
            if stripe {c.rect((cx as isize+sx) as usize,(cy as isize+dy) as usize,(end-sx) as usize,1,0xFF8290A0);}
            sx=end;
        }
    }
    c.ring(cx,cy,r,0xFFA8B3C0); c.ring(cx,cy,(r/4).max(3),0xFF263343);
    c.text(x+w/4,y+h*75/100,"NO COVER",0xFFB7C3D0,1);
}

fn render_toolbar_native(c:&mut NativeCanvas<'_>,mode:DisplayMode,popup:Option<ToolbarPopup>){
    c.rect(0,0,W,TOOLBAR_HEIGHT,0xFF0C1726); c.rect(0,TOOLBAR_HEIGHT-1,W,1,0xFF26374A);
    let buttons=[(12,100,"Open Disc"),(116,108,"Controller"),(229,92,"Paths"),(326,96,"Refresh"),(427,112,"Settings"),(676,80,"Exit")];
    for (i,(x,w,label)) in buttons.iter().enumerate(){
        let active=matches!((i,popup),(2,Some(ToolbarPopup::Paths))|(4,Some(ToolbarPopup::Display)));
        if active { c.round_rect(*x,5,*w,31,7,0xFF1B2D42); }
        let cx=*x+13;let cy=15;let color=if i==5{0xFFFF7777}else{0xFFB6C7DA};
        match i {
            0=>{c.round_rect(cx-7,cy-5,14,11,3,0xFF334B65);c.rect(cx-5,cy-1,10,5,0xFF0C1726);c.rect(cx-4,cy-5,6,3,0xFF334B65);}
            1=>{c.round_rect(cx-8,cy-5,16,11,4,0xFF344D67);c.rect(cx-5,cy-1,3,2,0xFF0C1726);c.rect(cx+3,cy-1,3,2,0xFF0C1726);c.rect(cx-1,cy+2,3,2,0xFF0C1726);}
            2=>{c.round_rect(cx-7,cy-6,14,13,2,0xFF344D67);c.rect(cx-4,cy-3,8,1,0xFF0C1726);c.rect(cx-4,cy,8,1,0xFF0C1726);c.rect(cx-4,cy+3,6,1,0xFF0C1726);}
            3=>{c.ring(cx,cy,7,0xFFB6C7DA);c.rect(cx+3,cy-7,5,2,0xFF0C1726);c.rect(cx+5,cy-5,2,5,0xFF0C1726);}
            4=>{c.ring(cx,cy,7,0xFFB6C7DA);c.ring(cx,cy,3,0xFFB6C7DA);for(dx,dy)in[(0,-10),(0,8),(-10,0),(8,0)]{c.rect((cx as isize+dx)as usize,(cy as isize+dy)as usize,2,2,0xFFB6C7DA);}}
            _=>{c.rect(cx-1,cy-8,3,12,color);c.rect(cx+2,cy+2,6,3,color);c.rect(cx+7,cy+4,2,4,color);}
        }
        c.text(*x+31,14,label,if i==5{0xFFFF808A}else{0xFFE0E8F1},1);
        if i!=5 { c.rect(*x+*w,11,1,18,0xFF213247); }
    }
    let _=mode;
    match popup{
        Some(ToolbarPopup::Paths)=>{c.round_rect(229,TOOLBAR_HEIGHT+4,228,144,9,0xFF17263A);for(n,label)in["Add Disc Folder","Set Cover Artwork","Set CD Artwork","Set Bezels","Download Full Artwork","Rescan Library"].iter().enumerate(){let y=TOOLBAR_HEIGHT+10+n*22;if n==0{c.round_rect(235,y-3,216,22,5,0xFF213650);}c.text(245,y+2,label,0xFFE6EDF5,1);}},
        Some(ToolbarPopup::Display)=>{c.round_rect(427,TOOLBAR_HEIGHT+4,206,78,9,0xFF17263A);for(n,label)in["Borderless Fullscreen","Decorated Fullscreen","Windowed"].iter().enumerate(){c.text(440,TOOLBAR_HEIGHT+10+n*22,label,0xFFE6EDF5,1);}},
        None=>{}
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
        let Some(art) = &items[index].cover else { continue; };
        let col = slot % layout.columns; let row = slot / layout.columns;
        let x = layout.left + col * (layout.cell_w + layout.gap) + 3;
        let y = layout.top + row * (layout.cell_h + layout.gap) + 2;
        let w = layout.cell_w.saturating_sub(6);
        let h = layout.cell_h.saturating_sub(42);
        let sx = |n: usize| vx + n * vw / W;
        let sy = |n: usize| vy + n * vh / H;
        blit_fit_scaled(dst, dst_w, dst_h, art, sx(x), sy(y), (x + w) * vw / W + vx - sx(x), (y + h) * vh / H + vy - sy(y));
    }
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

pub fn render_boot(dst: &mut [u32], title: &str, disc_art: Option<&Artwork>, frame: u64) {
    dst[..W * H].fill(0xFF000000);
    let angle = frame as f32 * 0.055;
    if let Some(art) = disc_art { blit_rotated(dst, art, W as i32 / 2, 254, 212, angle); }
    else { fallback_spinner(dst, W as i32 / 2, 254, 104, angle); }
    let clean: String = title.chars().take(38).collect();
    text(dst, 270, 410, &clean, 0xFFE6E6E6, 1);
    text(dst, 240, 444, "STARTING DISC PLEASE WAIT", 0xFF9B9B9B, 1);
}

pub fn render_toolbar(dst: &mut [u32], visible: bool, mode: DisplayMode, popup: Option<ToolbarPopup>) {
    if !visible { return; }
    rect(dst, 0, 0, W, TOOLBAR_HEIGHT, 0xFF0C1421);
    rect(dst, 0, TOOLBAR_HEIGHT - 1, W, 1, 0xFF354357);
    text(dst, 12, 12, "OPENCDI", 0xFFF2F4F7, 1);
    toolbar_button(dst, 84, 8, 96, "OPEN DISC", 0);
    toolbar_button(dst, 181, 8, 122, "CONTROLLER", 1);
    toolbar_button(dst, 304, 8, 82, "PATHS", 2);
    toolbar_button(dst, 387, 8, 88, "REFRESH", 3);
    toolbar_button(dst, 476, 8, 96, "DISPLAY", 4);
    toolbar_button(dst, 690, 8, 70, "EXIT", 5);
    let mode_label = match mode { DisplayMode::BorderlessFullscreen => "BORDERLESS", DisplayMode::DecoratedFullscreen => "FULLSCREEN", DisplayMode::Windowed => "WINDOWED" };
    text(dst, 590, 19, mode_label, 0xFF91A2B7, 1);
    match popup {
        Some(ToolbarPopup::Paths) => {
            rect(dst, 304, TOOLBAR_HEIGHT, 244, 132, 0xFF172334);
            rect(dst, 304, TOOLBAR_HEIGHT, 244, 1, 0xFF4A607C);
            for (n, label) in ["ADD DISC FOLDER", "SET COVER ART", "SET CD ART", "SET BEZELS", "DOWNLOAD FULL ART", "RESCAN LIBRARY"].iter().enumerate() {
                let y = TOOLBAR_HEIGHT + 5 + n * 21;
                text(dst, 314, y, label, 0xFFE2E8F0, 1);
            }
        }
        Some(ToolbarPopup::Display) => {
            rect(dst, 476, TOOLBAR_HEIGHT, 190, 71, 0xFF172334);
            rect(dst, 476, TOOLBAR_HEIGHT, 190, 1, 0xFF4A607C);
            for (n, label) in ["BORDERLESS FULLSCREEN", "DECORATED FULLSCREEN", "WINDOWED"].iter().enumerate() {
                text(dst, 486, TOOLBAR_HEIGHT + 7 + n * 21, label, 0xFFE2E8F0, 1);
            }
        }
        None => {}
    }
}

fn toolbar_button(dst: &mut [u32], x: usize, y: usize, w: usize, label: &str, kind: usize) {
    let icon_x = x + w / 2 - 7;
    let icon_y = y;
    let color = if kind == 5 { 0xFFFF7777 } else { 0xFF9ECFFF };
    match kind {
        0 | 2 => {
            rect(dst, icon_x, icon_y + 3, 14, 9, color);
            rect(dst, icon_x + 1, icon_y + 1, 6, 3, color);
            rect(dst, icon_x + 2, icon_y + 6, 10, 4, 0xFF0C1421);
        }
        1 => {
            rect(dst, icon_x + 1, icon_y + 3, 12, 8, color);
            rect(dst, icon_x + 3, icon_y + 5, 2, 2, 0xFF0C1421);
            rect(dst, icon_x + 10, icon_y + 5, 2, 2, 0xFF0C1421);
            rect(dst, icon_x + 5, icon_y + 7, 4, 2, 0xFF0C1421);
        }
        3 => {
            rect(dst, icon_x + 3, icon_y + 1, 8, 2, color);
            rect(dst, icon_x + 11, icon_y + 3, 2, 5, color);
            rect(dst, icon_x + 4, icon_y + 9, 8, 2, color);
            rect(dst, icon_x + 1, icon_y + 5, 2, 5, color);
        }
        4 => {
            rect(dst, icon_x + 1, icon_y + 1, 12, 9, color);
            rect(dst, icon_x + 3, icon_y + 3, 8, 5, 0xFF0C1421);
            rect(dst, icon_x + 6, icon_y + 10, 3, 2, color);
        }
        _ => {
            rect(dst, icon_x + 2, icon_y + 1, 2, 11, color);
            rect(dst, icon_x + 4, icon_y + 1, 8, 2, color);
            rect(dst, icon_x + 8, icon_y + 4, 2, 7, color);
            rect(dst, icon_x + 5, icon_y + 8, 5, 2, color);
        }
    }
    text(dst, x + w / 2 - label.len() * 3, y + 17, label, if kind == 5 { 0xFFFF9A9A } else { 0xFFD2DCE8 }, 1);
}

pub fn toolbar_hit(x: usize, y: usize, popup: Option<ToolbarPopup>) -> Option<ToolbarAction> {
    if let Some(popup) = popup {
        if popup == ToolbarPopup::Paths && (229..457).contains(&x) && (TOOLBAR_HEIGHT..TOOLBAR_HEIGHT + 148).contains(&y) {
            return match y.saturating_sub(TOOLBAR_HEIGHT + 10) / 22 {
                0 => Some(ToolbarAction::AddDiscFolder), 1 => Some(ToolbarAction::SetCovers), 2 => Some(ToolbarAction::SetCd),
                3 => Some(ToolbarAction::SetBezels), 4 => Some(ToolbarAction::DownloadArtwork), 5 => Some(ToolbarAction::Refresh), _ => None,
            };
        }
        if popup == ToolbarPopup::Display && (427..633).contains(&x) && (TOOLBAR_HEIGHT..TOOLBAR_HEIGHT + 82).contains(&y) {
            return match y.saturating_sub(TOOLBAR_HEIGHT + 8) / 22 { 0 => Some(ToolbarAction::Borderless), 1 => Some(ToolbarAction::Decorated), 2 => Some(ToolbarAction::Windowed), _ => None };
        }
    }
    if y >= TOOLBAR_HEIGHT { return None; }
    match x {
        12..=111 => Some(ToolbarAction::OpenDisc), 116..=224 => Some(ToolbarAction::Controller),
        229..=321 => Some(ToolbarAction::Paths), 326..=422 => Some(ToolbarAction::Refresh),
        427..=539 => Some(ToolbarAction::Display), 676..=760 => Some(ToolbarAction::Exit), _ => None,
    }
}

pub fn pause_menu_hit(x: usize, y: usize) -> Option<usize> {
    if !(214..554).contains(&x) { return None; }
    for row in 0..4 { let top = 234 + row * 42; if (top..top + 34).contains(&y) { return Some(row); } }
    None
}

pub fn pause_menu_hit_full(x: usize, y: usize, width: usize, height: usize) -> Option<usize> {
    if width==0||height==0{return None;}
    let bx=x.saturating_mul(W)/width;let by=y.saturating_mul(H)/height;
    if !(140..620).contains(&bx){return None;}
    let start=205usize;let step=48usize;let row_h=40usize;
    (0..5).find(|row|{let top=start+row*step;(top..top+row_h).contains(&by)})
}

pub fn render_pause_menu_full(dst: &mut [u32], width: usize, height: usize, title: &str, selected: usize) {
    if width == 0 || height == 0 || dst.len() < width.saturating_mul(height) { return; }
    let src=dst[..width*height].to_vec();
    for by in (0..height).step_by(8){for bx in (0..width).step_by(8){
        let mut r=0u32;let mut g=0u32;let mut b=0u32;let mut n=0u32;
        for yy in by..(by+8).min(height){for xx in bx..(bx+8).min(width){let p=src[yy*width+xx];r+=(p>>16)&255;g+=(p>>8)&255;b+=p&255;n+=1;}}
        let avg=0xFF000000|((r/n)<<16)|((g/n)<<8)|(b/n);let tinted=blend(0xFF091321,avg,172);
        rect_full(dst,width,height,bx,by,8,8,tinted);
    }}
    let mut c=NativeCanvas::overlay(dst,width,height,(0,0,width,height));
    c.round_rect(145,89,478,390,17,0x99000000);
    c.round_rect(140,84,478,390,17,0xFF111D2B);
    c.round_rect(140,84,478,3,2,0xFF63B6F5);
    c.ring(166,119,10,0xFF80C6F8);c.rect(163,113,3,12,0xFF80C6F8);c.rect(169,113,3,12,0xFF80C6F8);
    c.text(186,101,"Game Paused",0xFFF1F5FA,2);
    let clean:String=title.chars().take(40).collect();c.text(186,132,&clean,0xFFB6C9DD,1);
    c.text(163,160,"Press the pause button again to resume",0xFF91A6BC,1);
    let labels=["Resume","Save State","Load State","Controller Settings","Stop and Return to Library"];
    let icons=[0usize,1,2,3,4];
    for(row,label)in labels.iter().enumerate(){
        let y=205+row*48;let active=row==selected;
        if active{c.round_rect(154,y,450,40,8,0xFF2789CF);c.round_rect(154,y,4,40,2,0xFF82CEFF);}
        else{c.round_rect(154,y,450,40,8,0xFF1B2A3C);}
        let ic=if active{0xFFF4F9FE}else{0xFFB9CBE0};let ix=174;let iy=y+15;
        match icons[row]{
            0=>{c.rect(ix,iy-7,3,14,ic);c.rect(ix+7,iy-7,3,14,ic);},
            1=>{c.round_rect(ix-5,iy-7,16,15,3,ic);c.rect(ix-2,iy-4,10,5,0xFF1B2A3C);c.rect(ix-2,iy+4,10,4,0xFF1B2A3C);},
            2=>{c.round_rect(ix-5,iy-7,16,15,3,ic);c.rect(ix-2,iy-4,10,9,0xFF1B2A3C);c.rect(ix,iy-2,2,6,ic);c.rect(ix+4,iy-2,2,6,ic);},
            3=>{c.round_rect(ix-6,iy-6,18,13,6,ic);c.rect(ix-3,iy-1,3,2,0xFF1B2A3C);c.rect(ix+5,iy-1,3,2,0xFF1B2A3C);},
            _=>{c.rect(ix-2,iy-8,3,17,ic);c.rect(ix+4,iy-6,3,15,ic);c.rect(ix+10,iy-4,3,13,ic);}
        }
        c.text(201,y+12,label,if active{0xFFFFFFFF}else{0xFFE1E8F1},1);
    }
    c.line(160,455,438,0xFF2A3A4D);c.text(165,461,"Press Guide / Home again to resume",0xFF91A6BC,1);
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
    let (sin, cos) = angle.sin_cos(); let radius = size / 2;
    for oy in -radius..radius { for ox in -radius..radius {
        let u = ox as f32 * cos / size as f32 + oy as f32 * sin / size as f32 + 0.5;
        let v = -ox as f32 * sin / size as f32 + oy as f32 * cos / size as f32 + 0.5;
        if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) { continue; }
        let x = cx + ox; let y = cy + oy;
        if x < 0 || y < 0 || x as usize >= W || y as usize >= H { continue; }
        let src = sample_bilinear(art, u * art.w as f32 - 0.5, v * art.h as f32 - 0.5); let a = src >> 24;
        let idx = y as usize * W + x as usize; dst[idx] = blend(src, dst[idx], a);
    }}
}

fn fallback_spinner(dst: &mut [u32], cx: i32, cy: i32, radius: i32, angle: f32) {
    for y in -radius..radius { for x in -radius..radius {
        let d2 = x*x + y*y; if d2 > radius*radius || d2 < 11*11 { continue; }
        let a = (y as f32).atan2(x as f32); let diff = (a-angle).sin().abs();
        let radial = ((d2 as f32).sqrt() as i32 / 9) % 2;
        let shade = if diff < 0.14 { 220 } else if radial == 0 { 132 } else { 76 };
        let px=cx+x; let py=cy+y;
        if px >= 0 && py >= 0 && (px as usize) < W && (py as usize) < H {
            dst[py as usize * W + px as usize] = 0xFF000000 | (shade << 16) | (shade << 8) | shade;
        }
    }}
    rect(dst, (cx-12) as usize, (cy-12) as usize, 24, 24, 0xFF555B62);
    rect(dst, (cx-5) as usize, (cy-5) as usize, 10, 10, 0xFF000000);
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

fn sample_bilinear(art: &Artwork, x: f32, y: f32) -> u32 {
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

fn fallback_cover(dst: &mut [u32], x: usize, y: usize, w: usize, h: usize) {
    rect(dst, x, y, w, h, 0xFF303B49);
    rect(dst, x + 4, y + 4, w - 8, h - 8, 0xFF202B39);
    rect(dst, x + 10, y + 10, 3, h - 20, 0xFF596675);
    let cx = x + w / 2;
    let cy = y + h / 2 - 8;
    let r = 30i32;
    for dy in -r..=r { for dx in -r..=r {
        let d = dx * dx + dy * dy;
        if d <= r * r && d >= 9 * 9 {
            let px = (cx as i32 + dx) as usize; let py = (cy as i32 + dy) as usize;
            dst[py * W + px] = if (dx + dy).rem_euclid(9) < 2 { 0xFF96A3B0 } else { 0xFF536170 };
        }
    }}
    text(dst, x + 18, y + h - 29, "NO COVER", 0xFFB9C3CF, 1);
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
