//! Keys are never written next to a vault. Production macOS uses Keychain.
use crate::{domain::*,store::random_key};
use zeroize::Zeroizing;
use std::sync::Arc;

pub trait KeyProvider:Send+Sync {
    fn get(&self,id:&str)->Result<Zeroizing<[u8;32]>>;
    fn put(&self,id:&str,key:&[u8;32])->Result<()>;
    fn delete(&self,id:&str)->Result<()>;
}
pub struct SystemKeys;
const SERVICE:&str="com.cdna.desktop.vault.v1";
impl KeyProvider for SystemKeys {
    fn get(&self,id:&str)->Result<Zeroizing<[u8;32]>>{
        ensure(valid_uuid(id),"INPUT_INVALID")?;
        #[cfg(target_os="macos")]{
            let bytes=Zeroizing::new(security_framework::passwords::get_generic_password(SERVICE,id).map_err(|_|Error::new("KEYCHAIN_UNAVAILABLE"))?);
            ensure(bytes.len()==32,"VAULT_UNLOCK_FAILED")?;let mut key=Zeroizing::new([0u8;32]);key.copy_from_slice(&bytes);Ok(key)
        }
        #[cfg(not(target_os="macos"))]{Err(Error::new("UNSUPPORTED_PLATFORM_KEYSTORE"))}
    }
    fn put(&self,id:&str,key:&[u8;32])->Result<()>{
        ensure(valid_uuid(id),"INPUT_INVALID")?;
        #[cfg(target_os="macos")]{security_framework::passwords::set_generic_password(SERVICE,id,key).map_err(|_|Error::new("KEYCHAIN_UNAVAILABLE"))}
        #[cfg(not(target_os="macos"))]{let _=key;Err(Error::new("UNSUPPORTED_PLATFORM_KEYSTORE"))}
    }
    fn delete(&self,id:&str)->Result<()>{
        ensure(valid_uuid(id),"INPUT_INVALID")?;
        #[cfg(target_os="macos")]{security_framework::passwords::delete_generic_password(SERVICE,id).map_err(|_|Error::new("KEYCHAIN_UNAVAILABLE"))}
        #[cfg(not(target_os="macos"))]{Err(Error::new("UNSUPPORTED_PLATFORM_KEYSTORE"))}
    }
}
pub fn system()->Arc<dyn KeyProvider>{Arc::new(SystemKeys)}
/// Test-only volatile key store. It cannot reopen persistent user data after a
/// process restart and is deliberately unavailable in production desktop builds.
#[cfg(any(test,feature="dev-bridge"))]
#[derive(Default)]
pub struct MemoryKeys(std::sync::Mutex<std::collections::HashMap<String,Zeroizing<[u8;32]>>>);
#[cfg(any(test,feature="dev-bridge"))]
impl KeyProvider for MemoryKeys {
    fn get(&self,id:&str)->Result<Zeroizing<[u8;32]>>{self.0.lock().map_err(|_|Error::new("INTERNAL_ERROR"))?.get(id).map(|k|Zeroizing::new(**k)).ok_or_else(||Error::new("KEYCHAIN_UNAVAILABLE"))}
    fn put(&self,id:&str,key:&[u8;32])->Result<()>{self.0.lock().map_err(|_|Error::new("INTERNAL_ERROR"))?.insert(id.into(),Zeroizing::new(*key));Ok(())}
    fn delete(&self,id:&str)->Result<()>{self.0.lock().map_err(|_|Error::new("INTERNAL_ERROR"))?.remove(id);Ok(())}
}
pub fn fresh(keys:&dyn KeyProvider,id:&str)->Result<Zeroizing<[u8;32]>>{let key=random_key();keys.put(id,&key)?;Ok(key)}

/// MCP grants use a distinct Keychain service. The MCP binary has no API to fetch
/// a vault master key, even if somebody supplies a vault UUID as a connection ID.
pub fn store_mcp_token(connection_id:&str,token:&str)->Result<()> {
    ensure(valid_uuid(connection_id)&&token.len()==64&&token.bytes().all(|b|b.is_ascii_hexdigit()),"INPUT_INVALID")?;
    #[cfg(target_os="macos")]{security_framework::passwords::set_generic_password("com.cdna.desktop.mcp.v1",connection_id,token.as_bytes()).map_err(|_|Error::new("KEYCHAIN_UNAVAILABLE"))}
    #[cfg(not(target_os="macos"))]{Err(Error::new("UNSUPPORTED_PLATFORM_KEYSTORE"))}
}
pub fn load_mcp_token(connection_id:&str)->Result<Zeroizing<String>> {
    ensure(valid_uuid(connection_id),"INPUT_INVALID")?;
    #[cfg(target_os="macos")]{let bytes=Zeroizing::new(security_framework::passwords::get_generic_password("com.cdna.desktop.mcp.v1",connection_id).map_err(|_|Error::new("KEYCHAIN_UNAVAILABLE"))?);let s=std::str::from_utf8(&bytes).map_err(|_|Error::new("KEYCHAIN_UNAVAILABLE"))?;Ok(Zeroizing::new(s.to_owned()))}
    #[cfg(not(target_os="macos"))]{Err(Error::new("UNSUPPORTED_PLATFORM_KEYSTORE"))}
}
pub fn remove_mcp_token(connection_id:&str){
    #[cfg(target_os="macos")]{let _=security_framework::passwords::delete_generic_password("com.cdna.desktop.mcp.v1",connection_id);}
    #[cfg(not(target_os="macos"))]{let _=connection_id;}
}
