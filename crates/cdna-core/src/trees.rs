//! Optional LightGBM native engine. Uses the vendor C API, not a reimplemented tree.
//! Only a locally packaged, SHA-256 pinned library may be loaded.
use crate::domain::*;
use libloading::Library;
use sha2::{Digest,Sha256};
use std::{ffi::{CString,c_void,c_char,c_int,c_longlong},path::Path};

type Load=unsafe extern "C" fn(*const c_char,*mut c_int,*mut *mut c_void)->c_int;
type Predict=unsafe extern "C" fn(*mut c_void,*const c_void,c_int,i32,i32,c_int,c_int,c_int,c_int,*const c_char,*mut c_longlong,*mut f64)->c_int;
type Free=unsafe extern "C" fn(*mut c_void)->c_int;
pub fn score_native(path:&Path,expected_sha256:&str,model_text:&str,rows:&[Vec<f64>])->Result<Vec<f64>>{
    ensure(path.is_file()&&expected_sha256.len()==64,"MODEL_UNAVAILABLE")?;
    ensure(!std::fs::symlink_metadata(path)?.file_type().is_symlink(),"UNSAFE_PATH")?;
    ensure(hex::encode(Sha256::digest(std::fs::read(path)?))==expected_sha256,"LIBRARY_INTEGRITY_FAILED")?;
    ensure(!rows.is_empty()&&rows.len()<=100000&&rows[0].len()<=2048&&rows.iter().all(|r|r.len()==rows[0].len()&&r.iter().all(|n|n.is_finite())),"INPUT_INVALID")?;
    let text=CString::new(model_text).map_err(|_|Error::new("MODEL_INVALID"))?;
    let config=CString::new("num_threads=2").map_err(|_|Error::new("MODEL_INVALID"))?;
    let input:Vec<f64>=rows.iter().flatten().copied().collect();let mut output=vec![0.0;rows.len()];
    // The unsafe surface is limited to the documented vendor ABI and a verified,
    // packaged library. It is never exposed as a user/agent-selected path.
    unsafe{
        let lib=Library::new(path).map_err(|_|Error::new("MODEL_UNAVAILABLE"))?;
        let load=lib.get::<Load>(b"LGBM_BoosterLoadModelFromString\0").map_err(|_|Error::new("MODEL_UNAVAILABLE"))?;
        let predict=lib.get::<Predict>(b"LGBM_BoosterPredictForMat\0").map_err(|_|Error::new("MODEL_UNAVAILABLE"))?;
        let free=lib.get::<Free>(b"LGBM_BoosterFree\0").map_err(|_|Error::new("MODEL_UNAVAILABLE"))?;
        let mut iterations=0;let mut handle=std::ptr::null_mut();
        ensure(load(text.as_ptr(),&mut iterations,&mut handle)==0&&!handle.is_null(),"MODEL_INVALID")?;
        let mut count=0i64;
        let status=predict(handle,input.as_ptr().cast(),1,rows.len() as i32,rows[0].len() as i32,1,0,0,-1,config.as_ptr(),&mut count,output.as_mut_ptr());
        let _=free(handle);
        ensure(status==0&&count==rows.len() as i64&&output.iter().all(|v|v.is_finite()),"MODEL_INVALID")?;
    }
    Ok(output)
}
