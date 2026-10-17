//! Narrow RAII binding to Windows' system SQLite. No bundled runtime, Python, or shell commands.
use anyhow::{Context,Result,ensure};
use std::{ffi::{CString,CStr,c_char,c_int,c_void},path::Path,ptr::NonNull,sync::Arc};
type Open=unsafe extern "C" fn(*const c_char,*mut *mut c_void,c_int,*const c_char)->c_int;
type Close=unsafe extern "C" fn(*mut c_void)->c_int;
type Prepare=unsafe extern "C" fn(*mut c_void,*const c_char,c_int,*mut *mut c_void,*mut *const c_char)->c_int;
type Step=unsafe extern "C" fn(*mut c_void)->c_int;
type BindNull=unsafe extern "C" fn(*mut c_void,c_int)->c_int;
type BindInt=unsafe extern "C" fn(*mut c_void,c_int,i64)->c_int;
type BindDouble=unsafe extern "C" fn(*mut c_void,c_int,f64)->c_int;
type BindText=unsafe extern "C" fn(*mut c_void,c_int,*const c_char,c_int,Option<unsafe extern "C" fn(*mut c_void)>)->c_int;
type ColumnInt=unsafe extern "C" fn(*mut c_void,c_int)->i64;
type ColumnDouble=unsafe extern "C" fn(*mut c_void,c_int)->f64;
type ColumnNum=unsafe extern "C" fn(*mut c_void,c_int)->c_int;
type ColumnPtr=unsafe extern "C" fn(*mut c_void,c_int)->*const c_void;
type Error=unsafe extern "C" fn(*mut c_void)->*const c_char;
struct Api {_library:libloading::Library,open:Open,close:Close,prepare:Prepare,step:Step,finalize:Step,bind_null:BindNull,bind_int:BindInt,bind_double:BindDouble,bind_text:BindText,column_count:Step,column_type:ColumnNum,column_int:ColumnInt,column_double:ColumnDouble,column_text:ColumnPtr,column_blob:ColumnPtr,column_bytes:ColumnNum,error:Error}
impl Api {
    fn load()->Result<Self>{
        #[cfg(not(windows))] {anyhow::bail!("实验性 .lrcat 导出当前仅支持 Windows 10/11");}
        #[cfg(windows)] {
            // SAFETY: only the Windows system directory is searched; symbols below are the
            // documented SQLite C ABI, and the Library is retained for every connection.
            unsafe {
                let library:libloading::Library=libloading::os::windows::Library::load_with_flags("winsqlite3.dll",libloading::os::windows::LOAD_LIBRARY_SEARCH_SYSTEM32).context("Windows 系统 SQLite 不可用")?.into();
                macro_rules! get {($name:literal,$ty:ty)=>{*library.get::<$ty>(concat!($name,"\0").as_bytes())?};}
                Ok(Self{open:get!("sqlite3_open_v2",Open),close:get!("sqlite3_close_v2",Close),prepare:get!("sqlite3_prepare_v2",Prepare),step:get!("sqlite3_step",Step),finalize:get!("sqlite3_finalize",Step),bind_null:get!("sqlite3_bind_null",BindNull),bind_int:get!("sqlite3_bind_int64",BindInt),bind_double:get!("sqlite3_bind_double",BindDouble),bind_text:get!("sqlite3_bind_text",BindText),column_count:get!("sqlite3_column_count",Step),column_type:get!("sqlite3_column_type",ColumnNum),column_int:get!("sqlite3_column_int64",ColumnInt),column_double:get!("sqlite3_column_double",ColumnDouble),column_text:get!("sqlite3_column_text",ColumnPtr),column_blob:get!("sqlite3_column_blob",ColumnPtr),column_bytes:get!("sqlite3_column_bytes",ColumnNum),error:get!("sqlite3_errmsg",Error),_library:library})
            }
        }
    }
}
#[derive(Clone,Debug,PartialEq)]
pub(crate) enum Value {Null,Int(i64),Real(f64),Text(String),Blob(Vec<u8>)}
impl From<&str> for Value {fn from(s:&str)->Self{Self::Text(s.into())}}
impl From<String> for Value {fn from(s:String)->Self{Self::Text(s)}}
impl From<i64> for Value {fn from(n:i64)->Self{Self::Int(n)}}
impl Value {
    pub fn text(&self)->Result<&str>{match self{Self::Text(s)=>Ok(s),_=>anyhow::bail!("SQLite 字段不是文本")}}
}
pub(crate) struct Db {ptr:NonNull<c_void>,api:Arc<Api>}
struct Statement<'a>{ptr:NonNull<c_void>,db:&'a Db}
impl Drop for Statement<'_>{fn drop(&mut self){/* SAFETY: statement is exclusively owned and its connection/DLL outlive it. */unsafe{(self.db.api.finalize)(self.ptr.as_ptr());}}}
impl Drop for Db {fn drop(&mut self){/* SAFETY: no statement escapes query(); connection and DLL are still alive. */unsafe{(self.api.close)(self.ptr.as_ptr());}}}
impl Db {
    pub(crate) fn open(name:&str,flags:i32)->Result<Self>{
        let api=Arc::new(Api::load()?);let name=CString::new(name)?;let mut raw=std::ptr::null_mut();
        // SAFETY: the filename is NUL-terminated; SQLite initializes raw, with the default VFS.
        let code=unsafe{(api.open)(name.as_ptr(),&mut raw,flags|0x10000,std::ptr::null())};
        let ptr=NonNull::new(raw).context("SQLite 无法创建连接")?;let db=Self{ptr,api};ensure!(code==0,"无法打开 SQLite：{}",db.error());Ok(db)
    }
    pub fn read_only(path:&Path)->Result<Self>{
        let s=path.to_string_lossy();let normalized=if let Some(p)=s.strip_prefix("\\\\?\\UNC\\"){format!("\\\\{p}")}else{s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string()};
        let mut uri=url::Url::from_file_path(Path::new(&normalized)).map_err(|_|anyhow::anyhow!("目录路径必须为绝对路径"))?;uri.query_pairs_mut().append_pair("mode","ro").append_pair("immutable","1");
        let db=Self::open(uri.as_str(),1|0x40)?;db.query("PRAGMA query_only=ON",&[])?;Ok(db)
    }
    pub fn new(path:&Path)->Result<Self>{
        std::fs::OpenOptions::new().write(true).create_new(true).open(path)?.sync_all()?;
        let db=Self::open(path.to_str().context("SQLite 路径不是 Unicode")?,2)?;
        db.query("PRAGMA journal_mode=DELETE",&[])?;db.query("PRAGMA synchronous=FULL",&[])?;Ok(db)
    }
    fn error(&self)->String{
        // SAFETY: errmsg belongs to this live connection and is a NUL-terminated SQLite string.
        unsafe{let p=(self.api.error)(self.ptr.as_ptr());if p.is_null(){"SQLite error".into()}else{CStr::from_ptr(p).to_string_lossy().into_owned()}}
    }
    pub fn query(&self,sql:&str,params:&[Value])->Result<Vec<Vec<Value>>>{
        let sql=CString::new(sql)?;let mut raw=std::ptr::null_mut();let mut tail=std::ptr::null();
        // SAFETY: SQL lives until finalization; all out pointers are valid locals. SQLITE_STATIC
        // text bindings borrow params, which outlive the statement and are never mutated here.
        unsafe{
            let code=(self.api.prepare)(self.ptr.as_ptr(),sql.as_ptr(),-1,&mut raw,&mut tail);
            let statement=NonNull::new(raw).map(|ptr|Statement{ptr,db:self});
            ensure!(code==0,"SQLite SQL 失败：{}",self.error());
            ensure!(tail.is_null()||CStr::from_ptr(tail).to_bytes().iter().all(|b|b.is_ascii_whitespace()),"不接受多条 SQL 语句");
            let Some(statement)=statement else{return Ok(Vec::new())};let stmt=statement.ptr.as_ptr();
            for (n,v) in params.iter().enumerate(){let index=i32::try_from(n+1)?;let code=match v{
                Value::Null=>(self.api.bind_null)(stmt,index),Value::Int(n)=>(self.api.bind_int)(stmt,index,*n),Value::Real(n)=>{ensure!(n.is_finite(),"非有限 SQLite 数值");(self.api.bind_double)(stmt,index,*n)},Value::Text(s)=>(self.api.bind_text)(stmt,index,s.as_ptr().cast(),i32::try_from(s.len())?,None),Value::Blob(_)=>anyhow::bail!("不接受写入不透明 BLOB")};ensure!(code==0,"SQLite 参数失败：{}",self.error());}
            let columns=(self.api.column_count)(stmt);ensure!((0..4096).contains(&columns),"SQLite 列数过多");let mut rows=Vec::new();let mut bytes=0usize;
            loop{let code=(self.api.step)(stmt);if code==101{break}ensure!(code==100,"SQLite 执行失败：{}",self.error());let mut row=Vec::new();
                for i in 0..columns {row.push(match (self.api.column_type)(stmt,i){1=>Value::Int((self.api.column_int)(stmt,i)),2=>Value::Real((self.api.column_double)(stmt,i)),3|4=>{let kind=(self.api.column_type)(stmt,i);let ptr=if kind==3{(self.api.column_text)(stmt,i)}else{(self.api.column_blob)(stmt,i)};let n=(self.api.column_bytes)(stmt,i);ensure!((0..=16*1024*1024).contains(&n),"SQLite 字段过大");bytes=bytes.saturating_add(n as usize);ensure!(bytes<=256*1024*1024,"SQLite 查询结果过大");ensure!(n==0||!ptr.is_null(),"SQLite 字段不可读");let data=if n==0{&[][..]}else{std::slice::from_raw_parts(ptr.cast::<u8>(),n as usize)};if kind==3{Value::Text(std::str::from_utf8(data)?.to_string())}else{Value::Blob(data.to_vec())}},5=>Value::Null,_=>anyhow::bail!("未知 SQLite 类型")});}
                rows.push(row);ensure!(rows.len()<=1_000_000,"SQLite 查询行数过多");
            }Ok(rows)
        }
    }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test]fn bound_text_transactions_readonly_and_single_statement_are_enforced(){let t=tempfile::tempdir().unwrap();let p=t.path().join("临时.sqlite");let db=Db::new(&p).unwrap();db.query("CREATE TABLE data(n INTEGER, v TEXT)",&[]).unwrap();db.query("INSERT INTO data VALUES(?,?)",&[Value::Int(7),Value::Text("中文'); DROP TABLE data; --\0值".into())]).unwrap();assert_eq!(db.query("SELECT n,v FROM data",&[]).unwrap()[0][0],Value::Int(7));assert!(db.query("SELECT 1; DROP TABLE data",&[]).is_err());drop(db);let db=Db::read_only(&p).unwrap();assert!(db.query("DELETE FROM data",&[]).is_err());assert_eq!(db.query("PRAGMA integrity_check",&[]).unwrap()[0][0],Value::Text("ok".into()));}
}

