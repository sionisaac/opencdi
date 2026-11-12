//! Calendar helpers. Host time is opted into by the desktop frontend only;
//! headless emulation and replay tests retain their deterministic clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Calendar { pub year:u16, pub month:u8, pub day:u8, pub weekday:u8, pub hour:u8, pub minute:u8, pub second:u8 }
pub fn bcd(n:u8)->u8 { (n/10)*16+n%10 }
fn decimal(n:u8)->u8 { (n>>4)*10+(n&15) }
fn days(year:u16,month:u8)->u8 { match month { 2=>if year%4==0&&(year%100!=0||year%400==0){29}else{28},4|6|9|11=>30,_=>31 } }
impl Calendar {
    pub fn registers(self)->[u8;8] { [0,bcd(self.second),bcd(self.minute),bcd(self.hour),self.weekday,bcd(self.day),bcd(self.month),bcd((self.year%100) as u8)] }
    pub fn from_registers(r:[u8;8])->Option<Self> {
        let yy=decimal(r[7]);let c=Self{year:if yy>=70{1900+yy as u16}else{2000+yy as u16},month:decimal(r[6]&31),day:decimal(r[5]&63),weekday:r[4]&7,hour:decimal(r[3]&63),minute:decimal(r[2]&127),second:decimal(r[1]&127)};
        (c.month>=1&&c.month<=12&&c.day>=1&&c.day<=days(c.year,c.month)&&c.weekday>=1&&c.weekday<=7&&c.hour<24&&c.minute<60&&c.second<60).then_some(c)
    }
    pub fn advance(&mut self,seconds:u64) {
        let total=self.hour as u64*3600+self.minute as u64*60+self.second as u64+seconds;
        self.hour=((total%86400)/3600) as u8;self.minute=((total%3600)/60) as u8;self.second=(total%60) as u8;
        for _ in 0..total/86400 {self.weekday=self.weekday%7+1;self.day+=1;if self.day>days(self.year,self.month){self.day=1;self.month+=1;if self.month>12{self.month=1;self.year+=1;}}}
    }
}
pub fn set(bus:&mut crate::bus::CdiBus,time:Calendar) {
    let regs=time.registers();
    if let Some(m)=bus.mono4.as_deref_mut(){m.nvram.clock.regs=regs;}else{bus.nvram.ram[8184..8192].copy_from_slice(&regs);}
}
pub fn advance(bus:&mut crate::bus::CdiBus,seconds:u64) {
    if let Some(m)=bus.mono4.as_deref_mut(){
        // Do not change a serial clock transaction halfway through a read/write.
        if m.nvram.clock.matched==64 || m.nvram.clock.regs[4]&0x20!=0{return;}
        let old=m.nvram.clock.regs;
        if let Some(mut c)=Calendar::from_registers(old){c.advance(seconds);let mut r=c.registers();r[0]=old[0];r[4]|=old[4]&!7;m.nvram.clock.regs=r;}
    }else{
        if bus.nvram.ram[8184]&0xC0!=0||bus.nvram.ram[8185]&0x80!=0{return;}
        let old:[u8;8]=bus.nvram.ram[8184..8192].try_into().unwrap();
        if let Some(mut c)=Calendar::from_registers(old){c.advance(seconds);let mut r=c.registers();r[0]=old[0];r[4]|=old[4]&!7;bus.nvram.ram[8184..8192].copy_from_slice(&r);}
    }
}
#[cfg(windows)]
pub fn local_now()->Option<Calendar>{
    #[repr(C)] #[derive(Default)] struct SystemTime{year:u16,month:u16,weekday:u16,day:u16,hour:u16,minute:u16,second:u16,millis:u16}
    #[link(name="kernel32")] unsafe extern "system"{fn GetLocalTime(value:*mut SystemTime);}
    let mut t=SystemTime::default();unsafe{GetLocalTime(&mut t);}
    Some(Calendar{year:t.year,month:t.month as u8,day:t.day as u8,weekday:t.weekday as u8+1,hour:t.hour as u8,minute:t.minute as u8,second:t.second as u8})
}
#[cfg(not(windows))]
pub fn local_now()->Option<Calendar>{
    let out=std::process::Command::new("date").arg("+%Y %m %d %w %H %M %S").output().ok()?;
    let text=String::from_utf8(out.stdout).ok()?;let v:Vec<u16>=text.split_whitespace().map(str::parse).collect::<Result<_,_>>().ok()?;
    if v.len()!=7{return None;}Some(Calendar{year:v[0],month:v[1] as u8,day:v[2] as u8,weekday:v[3] as u8+1,hour:v[4] as u8,minute:v[5] as u8,second:v[6] as u8})
}
#[cfg(test)] mod tests{
    use super::*;
    #[test]fn calendar_carries_dates_and_leap_years(){let mut c=Calendar{year:2024,month:2,day:28,weekday:4,hour:23,minute:59,second:59};c.advance(1);assert_eq!((c.month,c.day,c.hour),(2,29,0));c.advance(86400);assert_eq!((c.month,c.day),(3,1));c.year=2026;c.month=12;c.day=31;c.advance(86400);assert_eq!((c.year,c.month,c.day),(2027,1,1));}
    #[test]fn both_boards_tick_and_core_defaults_stay_fixed(){let t=Calendar{year:2026,month:11,day:12,weekday:5,hour:23,minute:59,second:59};for board in [crate::bus::BoardKind::Mono1,crate::bus::BoardKind::Mono4]{let mut b=crate::bus::CdiBus::new();b.set_board(board);set(&mut b,t);advance(&mut b,1);let r=if let Some(m)=b.mono4.as_ref(){m.nvram.clock.regs}else{b.nvram.ram[8184..8192].try_into().unwrap()};let c=Calendar::from_registers(r).unwrap();assert_eq!((c.day,c.hour,c.second),(13,0,0));}assert_eq!(crate::mono4::nvram::Ds1216::default().regs,crate::mono4::nvram::DS1216_FIXED_TIME);}
}
