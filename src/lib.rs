// SPDX-License-Identifier: MIT
// SigmaOS-efivar - UEFI Variable Access component
// efivar-style UEFI variable read/write/list for SigmaOS,
// inspired by Arch's efivar package.

use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

// ============================================================================
// EFI GUIDs
// ============================================================================

/// Well-known EFI GUIDs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EfiGuid(pub [u8; 16]);

impl EfiGuid {
    /// EFI global variable GUID
    pub const GLOBAL: EfiGuid = EfiGuid([
        0x8b, 0xe4, 0xdf, 0x61, 0x93, 0xca, 0x11, 0xd2,
        0xaa, 0x0d, 0x00, 0xe0, 0x98, 0x03, 0x2b, 0x8c,
    ]);
    /// EFI image security database GUID
    pub const IMAGE_SECURITY: EfiGuid = EfiGuid([
        0xd7, 0x19, 0xb2, 0xcb, 0x3d, 0x3a, 0x45, 0x44,
        0x96, 0x85, 0x2d, 0x72, 0x87, 0xac, 0x00, 0xe6,
    ]);
    /// EFI shim GUID
    pub const SHIM: EfiGuid = EfiGuid([
        0x4a, 0x67, 0xb0, 0x82, 0x0a, 0x4c, 0x41, 0xcf,
        0x84, 0x2b, 0x23, 0x26, 0x72, 0x3c, 0x66, 0x96,
    ]);
    /// EFI systemd-boot GUID
    pub const SYSTEMD_BOOT: EfiGuid = EfiGuid([
        0x4a, 0x67, 0xb0, 0x82, 0x0a, 0x4c, 0x41, 0xcf,
        0x84, 0x2b, 0x23, 0x26, 0x72, 0x3c, 0x66, 0x96,
    ]);

    pub fn to_string(&self) -> String {
        // EFI GUID format: xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx
        format!("{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.0[3], self.0[2], self.0[1], self.0[0], // little-endian first fields
            self.0[5], self.0[4],
            self.0[7], self.0[6],
            self.0[8], self.0[9],
            self.0[10], self.0[11], self.0[12], self.0[13], self.0[14], self.0[15])
    }
}

// ============================================================================
// EFI VARIABLE
// ============================================================================

/// EFI variable attributes
pub mod attr {
    pub const NON_VOLATILE: u32 = 0x0000_0001;
    pub const BOOTSERVICE_ACCESS: u32 = 0x0000_0002;
    pub const RUNTIME_ACCESS: u32 = 0x0000_0004;
    pub const READ_ONLY: u32 = 0x0000_0008;
    pub const ERROR_RETURN: u32 = 0x0000_0010;
    pub const TIME_BASED_AUTHENTICATED_WRITE_ACCESS: u32 = 0x0000_0020;
    pub const APPEND_WRITE: u32 = 0x0000_0040;
}

/// An EFI variable
#[derive(Debug, Clone)]
pub struct EfiVariable {
    /// Variable name (e.g. "BootOrder", "SecureBoot")
    pub name: String,
    /// Variable GUID
    pub guid: EfiGuid,
    /// Attributes
    pub attributes: u32,
    /// Variable data
    pub data: Vec<u8>,
    /// When last modified (ms since epoch)
    pub timestamp_ms: u64,
}

impl EfiVariable {
    /// Whether variable is non-volatile
    pub fn is_nonvolatile(&self) -> bool {
        self.attributes & attr::NON_VOLATILE != 0
    }

    /// Whether variable is read-only
    pub fn is_readonly(&self) -> bool {
        self.attributes & attr::READ_ONLY != 0
    }

    /// Whether variable is append-only
    pub fn is_append(&self) -> bool {
        self.attributes & attr::APPEND_WRITE != 0
    }

    /// Parse u16 array (for BootOrder, Boot####)
    pub fn as_u16_array(&self) -> Vec<u16> {
        self.data.chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect()
    }

    /// Parse as UTF-16 string
    pub fn as_utf16(&self) -> Option<String> {
        let chars: Vec<u16> = self.data.chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16(&chars).ok()
    }

    /// Parse as UTF-8 string
    pub fn as_utf8(&self) -> Option<String> {
        String::from_utf8(self.data.clone()).ok()
    }

    /// Parse as u8
    pub fn as_u8(&self) -> Option<u8> {
        if self.data.len() == 1 { Some(self.data[0]) } else { None }
    }

    /// Parse as u32
    pub fn as_u32(&self) -> Option<u32> {
        if self.data.len() == 4 {
            Some(u32::from_le_bytes([self.data[0], self.data[1], self.data[2], self.data[3]]))
        } else { None }
    }
}

// ============================================================================
// EFI VARIABLE STORE
// ============================================================================

/// Well-known EFI variable names
pub mod names {
    pub const BOOT_ORDER: &str = "BootOrder";
    pub const BOOT_NEXT: &str = "BootNext";
    pub const BOOT_CURRENT: &str = "BootCurrent";
    pub const SECURE_BOOT: &str = "SecureBoot";
    pub const SETUP_MODE: &str = "SetupMode";
    pub const PLATFORM_LANG: &str = "PlatformLang";
    pub const CON_IN: &str = "ConIn";
    pub const CON_OUT: &str = "ConOut";
    pub const ERR_OUT: &str = "ErrOut";
    pub const DRIVER_ORDER: &str = "DriverOrder";
    pub const KEY_OPTION: &str = "KeyOption";
    pub const PLATFORM_KEY: &str = "PlatformKey";
    pub const KEY_EXCHANGE_KEY: &str = "KEK";
    pub const DB: &str = "db";
    pub const DBX: &str = "dbx";
    pub const VENDOR_KEYS: &str = "VendorKeys";
    pub const SHIM_LOCK: &str = "ShimLock";
    pub const MOK_LIST: &str = "MokList";
    pub const MOK_NEW: &str = "MokNew";
    pub const MOK_XNEW: &str = "MokXNew";
}

/// EFI variable store (simulated NVRAM)
pub struct EfiStore {
    /// Variables: (name, guid) -> variable
    vars: Mutex<HashMap<(String, EfiGuid), EfiVariable>>,
}

impl EfiStore {
    pub fn new() -> Self {
        Self {
            vars: Mutex::new(HashMap::new()),
        }
    }

    /// Set a variable
    pub fn set(&self, name: &str, guid: EfiGuid, attributes: u32, data: Vec<u8>) -> Result<(), EfiError> {
        // Check if existing variable is read-only
        let vars = self.vars.lock().unwrap();
        let key = (name.to_string(), guid);
        if let Some(existing) = vars.get(&key) {
            if existing.is_readonly() {
                return Err(EfiError::ReadOnly(name.to_string()));
            }
            // Check append-only: can only append
            if existing.is_append() && !data.starts_with(&existing.data) {
                return Err(EfiError::AppendOnly(name.to_string()));
            }
        }
        drop(vars);

        let var = EfiVariable {
            name: name.to_string(),
            guid,
            attributes,
            data,
            timestamp_ms: now_ms(),
        };
        self.vars.lock().unwrap().insert(key, var);
        Ok(())
    }

    /// Get a variable
    pub fn get(&self, name: &str, guid: EfiGuid) -> Option<EfiVariable> {
        self.vars.lock().unwrap().get(&(name.to_string(), guid)).cloned()
    }

    /// Get a variable by name only (searches all GUIDs)
    pub fn get_by_name(&self, name: &str) -> Option<EfiVariable> {
        self.vars.lock().unwrap().values()
            .find(|v| v.name == name)
            .cloned()
    }

    /// Delete a variable
    pub fn delete(&self, name: &str, guid: EfiGuid) -> Result<(), EfiError> {
        let mut vars = self.vars.lock().unwrap();
        let key = (name.to_string(), guid);
        if let Some(var) = vars.get(&key) {
            if var.is_readonly() {
                return Err(EfiError::ReadOnly(name.to_string()));
            }
        }
        vars.remove(&key);
        Ok(())
    }

    /// List all variables
    pub fn list(&self) -> Vec<EfiVariable> {
        self.vars.lock().unwrap().values().cloned().collect()
    }

    /// List variables for a GUID
    pub fn list_by_guid(&self, guid: EfiGuid) -> Vec<EfiVariable> {
        self.vars.lock().unwrap().values()
            .filter(|v| v.guid == guid)
            .cloned()
            .collect()
    }

    /// List variables by name prefix
    pub fn list_by_prefix(&self, prefix: &str) -> Vec<EfiVariable> {
        self.vars.lock().unwrap().values()
            .filter(|v| v.name.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// Get BootOrder as list of boot entry numbers
    pub fn boot_order(&self) -> Vec<u16> {
        self.get(names::BOOT_ORDER, EfiGuid::GLOBAL)
            .map(|v| v.as_u16_array())
            .unwrap_or_default()
    }

    /// Set BootOrder
    pub fn set_boot_order(&self, order: &[u16]) -> Result<(), EfiError> {
        let data: Vec<u8> = order.iter()
            .flat_map(|n| n.to_le_bytes())
            .collect();
        self.set(names::BOOT_ORDER, EfiGuid::GLOBAL, attr::NON_VOLATILE | attr::BOOTSERVICE_ACCESS, data)
    }

    /// Get SecureBoot state
    pub fn secure_boot_enabled(&self) -> bool {
        self.get(names::SECURE_BOOT, EfiGuid::GLOBAL)
            .and_then(|v| v.as_u8())
            .map(|v| v != 0)
            .unwrap_or(false)
    }

    /// Set SecureBoot state (requires setup mode)
    pub fn set_secure_boot(&self, enabled: bool) -> Result<(), EfiError> {
        if !self.setup_mode() && !enabled {
            return Err(EfiError::SecureBootLocked);
        }
        self.set(names::SECURE_BOOT, EfiGuid::GLOBAL,
            attr::NON_VOLATILE | attr::BOOTSERVICE_ACCESS, vec![if enabled { 1 } else { 0 }])
    }

    /// Check if in setup mode
    pub fn setup_mode(&self) -> bool {
        self.get(names::SETUP_MODE, EfiGuid::GLOBAL)
            .and_then(|v| v.as_u8())
            .map(|v| v != 0)
            .unwrap_or(true) // default to setup mode
    }

    /// Get a boot entry by number (Boot####)
    pub fn boot_entry(&self, num: u16) -> Option<EfiVariable> {
        let name = format!("Boot{:04X}", num);
        self.get(&name, EfiGuid::GLOBAL)
    }

    /// Set a boot entry
    pub fn set_boot_entry(&self, num: u16, description: &str, device_path: &[u8]) -> Result<(), EfiError> {
        let name = format!("Boot{:04X}", num);
        // EFI_LOAD_OPTION: attributes(4) + file_path_list_length(2) + description + device_path + optional_data
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_le_bytes()); // LOAD_OPTION_ACTIVE
        data.extend_from_slice(&(device_path.len() as u16).to_le_bytes());
        // UTF-16 description
        for c in description.encode_utf16() {
            data.extend_from_slice(&c.to_le_bytes());
        }
        data.push(0); data.push(0); // null terminator
        data.extend_from_slice(device_path);
        self.set(&name, EfiGuid::GLOBAL,
            attr::NON_VOLATILE | attr::BOOTSERVICE_ACCESS, data)
    }
}

impl Default for EfiStore {
    fn default() -> Self { Self::new() }
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

#[derive(Debug)]
pub enum EfiError {
    ReadOnly(String),
    AppendOnly(String),
    SecureBootLocked,
    InvalidData(String),
    NotFound(String),
}

impl fmt::Display for EfiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReadOnly(n) => write!(f, "variable is read-only: {}", n),
            Self::AppendOnly(n) => write!(f, "variable is append-only: {}", n),
            Self::SecureBootLocked => write!(f, "SecureBoot is locked"),
            Self::InvalidData(d) => write!(f, "invalid data: {}", d),
            Self::NotFound(n) => write!(f, "variable not found: {}", n),
        }
    }
}
impl std::error::Error for EfiError {}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_get_variable() {
        let store = EfiStore::new();
        store.set("TestVar", EfiGuid::GLOBAL, attr::NON_VOLATILE, vec![1, 2, 3]).unwrap();
        let var = store.get("TestVar", EfiGuid::GLOBAL).unwrap();
        assert_eq!(var.data, vec![1, 2, 3]);
        assert!(var.is_nonvolatile());
    }

    #[test]
    fn test_readonly_variable() {
        let store = EfiStore::new();
        store.set("ReadOnly", EfiGuid::GLOBAL, attr::READ_ONLY, vec![42]).unwrap();
        assert!(store.set("ReadOnly", EfiGuid::GLOBAL, attr::READ_ONLY, vec![43]).is_err());
        assert!(store.delete("ReadOnly", EfiGuid::GLOBAL).is_err());
    }

    #[test]
    fn test_append_only() {
        let store = EfiStore::new();
        store.set("AppendVar", EfiGuid::GLOBAL, attr::APPEND_WRITE, vec![1, 2]).unwrap();
        // Can append
        store.set("AppendVar", EfiGuid::GLOBAL, attr::APPEND_WRITE, vec![1, 2, 3]).unwrap();
        // Cannot overwrite
        assert!(store.set("AppendVar", EfiGuid::GLOBAL, attr::APPEND_WRITE, vec![9, 9]).is_err());
    }

    #[test]
    fn test_boot_order() {
        let store = EfiStore::new();
        store.set_boot_order(&[1, 2, 3]).unwrap();
        assert_eq!(store.boot_order(), vec![1, 2, 3]);
    }

    #[test]
    fn test_secure_boot() {
        let store = EfiStore::new();
        assert!(store.setup_mode()); // default
        store.set_secure_boot(true).unwrap();
        assert!(store.secure_boot_enabled());
    }

    #[test]
    fn test_guid_format() {
        let guid = EfiGuid::GLOBAL;
        let s = guid.to_string();
        assert_eq!(s.len(), 36); // UUID format
        assert!(s.contains('-'));
    }

    #[test]
    fn test_utf16_decode() {
        let mut data = Vec::new();
        for c in "Hello".encode_utf16() {
            data.extend_from_slice(&c.to_le_bytes());
        }
        let var = EfiVariable {
            name: "test".into(), guid: EfiGuid::GLOBAL,
            attributes: 0, data, timestamp_ms: 0,
        };
        assert_eq!(var.as_utf16(), Some("Hello".to_string()));
    }

    #[test]
    fn test_list_by_prefix() {
        let store = EfiStore::new();
        store.set("Boot0001", EfiGuid::GLOBAL, attr::NON_VOLATILE, vec![]).unwrap();
        store.set("Boot0002", EfiGuid::GLOBAL, attr::NON_VOLATILE, vec![]).unwrap();
        store.set("Driver0001", EfiGuid::GLOBAL, attr::NON_VOLATILE, vec![]).unwrap();
        let boots = store.list_by_prefix("Boot");
        assert_eq!(boots.len(), 2);
    }

    #[test]
    fn test_set_boot_entry() {
        let store = EfiStore::new();
        store.set_boot_entry(1, "Linux", &[0x01, 0x02]).unwrap();
        let entry = store.boot_entry(1).unwrap();
        assert!(entry.name == "Boot0001");
    }
}
