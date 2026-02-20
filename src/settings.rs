//#![feature(os_str_display)]
use std::{fmt, env, ffi::os_str::Display, fs::File, io::Write, path::PathBuf};

use aes_gcm::{ // cargo add aes-gcm
    aead::{KeyInit, OsRng},
    Aes256Gcm
};

use crate::crypt;
use crate::crypt::generate_key;

const PROGRAM: &str = "dejacmd";

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Settings
{
   // #[serde(skip)] program: String,
   #[serde(default = "Settings::default_local_database_url")]
   local_database_url:                 String,
   #[serde(skip_serializing_if = "Option::is_none")]
   local_user:                         Option<String>,
   #[serde(skip_serializing_if = "Option::is_none")]
   local_encrypted_password:           Option<String>,

   #[serde(skip_serializing_if = "Option::is_none")]
   central_database_url:               Option<String>,
   #[serde(skip_serializing_if = "Option::is_none")]
   central_user:                       Option<String>,
   #[serde(skip_serializing_if = "Option::is_none")]
   central_encrypted_password:         Option<String>,
   #[serde(skip_serializing_if = "Option::is_none")]
   encryption_key:                     Option<String>,

   #[serde(skip_serializing_if = "Option::is_none")]
   pub last_local_update_file:         Option<String>,
   #[serde(skip_serializing_if = "Option::is_none")]
   pub last_central_update_file:       Option<String>,
}

impl Default for Settings
{
   fn default() -> Self
//------------------
   {
      Self
      {
         local_database_url: Settings::default_local_database_url(),
         local_user: None,
         local_encrypted_password: None,
         central_database_url: None,
         central_user: None,
         central_encrypted_password: None,
         encryption_key: None,
         last_local_update_file: None,
         last_central_update_file: None,
      }
   }
}

impl fmt::Display for Settings
//-----------------------------
{
   fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result
   {
      let settings = match serde_json::to_string_pretty(&self)
      {
         | Ok(json) => json,
         | Err(_) => format!("local_database_url: {}, local_user: {:?}, central_database_url: {:?}, central_user: {:?}",
                            self.local_database_url, self.local_user, self.central_database_url, self.central_user)
      };

      let settings_file = match Settings::get_settings_path()
      {
         | Ok(p) => "(".to_string() + p.display().to_string().as_str() + ")",
         | Err(_) => "".to_string()
       };
      write!(f, "Settings {}\n{}\nLocal database URL: {}, Local_user: {:?}\nCentral Database URL: {:?}, Central User: {:?}",
             settings_file, settings, self.local_database_url, self.local_user, self.central_database_url, self.central_user)
   }
}

impl Settings
//===========
{
   fn default_local_database_path() -> PathBuf
   //----------------------------------------
   {
      let home_dir = Settings::get_home_dir();
      if env::consts::OS == "windows"
      {
         return home_dir.join(format!("{}.sqlite", PROGRAM));
      }
      home_dir.join(format!(".{}.sqlite", PROGRAM))
   }

   fn default_local_database_url() -> String
   //------------------------------------
   {
      let database_path = Settings::default_local_database_path();
      format!("sqlite://{}", database_path.display())
   }

   pub fn new() -> Self { Settings::default() }

   pub fn get_settings(&self) -> Result<Settings, String>
//-------------------------------------------
   {
      let settings_path = match Settings::get_settings_path()
      {
         | Ok(p) => p,
         | Err(_e) => match Settings::write_default_settings()
         {
            | Ok(pp) => pp,
            | Err(e) =>
            {
               let errmsg = format!("Error on get settings: {}", e);
               return Err(errmsg);
            }
         },
      };

      if !settings_path.exists()
      {
         match Settings::write_default_settings()
         {
            | Ok(_) => (),
            | Err(e) =>
            {
               eprintln!("Error creating default settings: {}", e);
               // PathBuf::new()
            }
         };
      }
      Ok(self.read_settings())
   }

   pub fn get_settings_or_default(&mut self) -> Settings
   //-------------------------------------------
   {
      match self.get_settings()
      {
         | Ok(mut s) => //s,
         {  //TODO: Moving encryption key to separate file - remove later
            match s.encryption_key.clone()
            {
               |  Some(k) =>
                  {
                     match s.set_encrypt_key(Some(k.clone()))
                     {
                        | Ok(_) =>
                        {
                           s.encryption_key = None;
                           match s.write_settings()
                           {
                              | Ok(_) => (),
                              | Err(e) =>
                              {
                                 s.encryption_key = Some(k);
                                 eprintln!("Error writing settings after moving encryption key: {}", e);
                              }
                           }
                        },
                        | Err(e) =>
                        {
                           eprintln!("Error moving encryption key to separate file: {}", e);
                        }
                     }
                  },
                  None => { }
            };
            *self = s.clone();
            s
         }
         | Err(_) => 
         {
            let s = Settings::default();
            *self = s.clone();
            s
         }
      }
   }

   fn set_encrypt_key(&mut self, hex_key: Option<String>) -> Result<(), String>
   //--------------------------------------------------------------------------
   {
      let encryption_file_path = match Settings::get_config_path()
      {
         | Ok(mut p) =>
         {
            p.push("encryption-key");
            p
         }
         | Err(e) =>
         {
            let errmsg = format!("Failed to get config path for encryption key: {}", e);
            eprintln!("{errmsg}");
            return Err(errmsg);
         }
      };
      let key = match hex_key
      {
         | Some(k) => k,
         | None => generate_key()
      };
      if key.trim().is_empty()
      {
         return Err("Encryption key cannot be empty".to_string());
      }
      match std::fs::write(&encryption_file_path, &key)
      {
         | Ok(_) => (),
         | Err(e) =>
         {
            let errmsg = format!("Failed to write encryption key to file {}: {}", encryption_file_path.display(), e);
            eprintln!("{errmsg}");
            return Err(errmsg);
         }
      };
      // Restrict file permissions to owner read/write only
      #[cfg(unix)]
      {
         use std::os::unix::fs::PermissionsExt;
         let perms = std::fs::Permissions::from_mode(0o600);
         let _ = std::fs::set_permissions(&encryption_file_path, perms);
      }
      #[cfg(windows)]
      {
         // Use icacls to remove inherited permissions and grant only the current user full control
         if let Ok(username) = env::var("USERNAME")
         {
            let path_str = encryption_file_path.display().to_string();
            let _ = std::process::Command::new("icacls")
               .args([&path_str, "/inheritance:r", "/grant:r", &format!("{}:(R,W)", username)])
               .output();
         }
      }
      Ok(())
   }

   fn get_encryption_key(is_generate: bool) -> Result<String, String>
   //-----------------------------------------------
   {
      // Read encryption key from hidden file encryption-key with read permissions only for current user
      let encryption_file_path = match Settings::get_config_path()
      {
         | Ok(mut p) =>
         {
            p.push("encryption-key");
            p
         }
         | Err(e) =>
         {
            let errmsg = format!("Failed to get config path for encryption key: {}", e);
            eprintln!("{errmsg}");
            return Err(errmsg);
         }
      };
      if !encryption_file_path.exists() && is_generate
      {
         // generate_key() already returns a hex-encoded string (64 hex chars = 32 bytes)
         let hex_key = generate_key();
         match std::fs::write(&encryption_file_path, &hex_key)
         {
            | Ok(_) => (),
            | Err(e) =>
            {
               let errmsg = format!("Failed to write encryption key to file {}: {}", encryption_file_path.display(), e);
               eprintln!("{errmsg}");
               return Err(errmsg);
            }
         };
         // Restrict file permissions to owner read/write only
         #[cfg(unix)]
         {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            let _ = std::fs::set_permissions(&encryption_file_path, perms);
         }
         #[cfg(windows)]
         {
            // Use icacls to remove inherited permissions and grant only the current user full control
            if let Ok(username) = env::var("USERNAME")
            {
               let path_str = encryption_file_path.display().to_string();
               let _ = std::process::Command::new("icacls")
                  .args([&path_str, "/inheritance:r", "/grant:r", &format!("{}:(R,W)", username)])
                  .output();
            }
         }
         Ok(hex_key)
      }
      else if encryption_file_path.exists()
      {
         let hex_key = match std::fs::read_to_string(&encryption_file_path)
         {
            | Ok(key) => key.trim().to_string(),
            | Err(e) =>
            {
               let errmsg = format!("Failed to read encryption key from file {}: {}", encryption_file_path.display(), e);
               eprintln!("{errmsg}");
               return Err(errmsg);
            }
         };
         Ok(hex_key)
      }
      else
      {
         Err("Encryption key file does not exist".to_string())
      }
      
   }

   pub fn write_settings(&self) -> Result<PathBuf, std::io::Error>
//-----------------------------------------------------------------------
   {
      let settings_path = match Settings::get_settings_path()
      {
         | Ok(p) => p,
         | Err(_) => match Settings::write_default_settings()
         {
            | Ok(pp) => pp,
            | Err(e) =>
            {
               // println!("Error on get settings: {}", e);
               return Err(e);
            }
         },
      };
      let mut file = File::create(&settings_path)?;
      for retry in 0..3
      {
         match file.try_lock()
         {
            | Ok(_) => break,
            | Err(e) =>
            {
               if retry == 2
               {
                  let errmsg = format!("Failed to lock settings file {}: {}", settings_path.display(), e);
                  // println!("{errmsg}");
                  return Err(std::io::Error::other(errmsg));
               }
               std::thread::sleep(std::time::Duration::from_millis(500));
            }
         }
      }
      let json = serde_json::to_string_pretty(&self)?;
      file.write_all(json.as_bytes())?;
      // println!("Wrote settings {} to {}", json, settings_path.display());
      Ok(settings_path)
   }

   pub fn set_local_database_url(&mut self, url: &str)
   //----------------------------------------
   {
      self.local_database_url = url.to_string();
   }

   pub fn get_local_database_url(&self) -> String
   //--------------------------------------
   {
      self.local_database_url.clone()
   }

   pub fn set_central_database_url(&mut self, url: &str)
   //-----------------------------------------
   {
      if url.trim().is_empty()
      {
         self.central_database_url = None;
      }
      else
      {
         self.central_database_url = Some(url.to_string());
      }
   }

   pub fn get_central_database_url(&self) -> String
   //---------------------------------------
   {
      self.central_database_url.clone().unwrap_or_default()
   }

   pub fn get_credentials(&self, is_local: bool) -> Result<(String, String), String>
   //-------------------------------------------------------
   {
      let user: String;
      let encrypted_password: String;
      if is_local
      {
         user = self.local_user.clone().unwrap_or_default();
         encrypted_password = self.local_encrypted_password.clone().unwrap_or_default();
      }
      else
      {
         user = self.central_user.clone().unwrap_or_default();
         encrypted_password = self.central_encrypted_password.clone().unwrap_or_default();
      }
      if encrypted_password.trim().is_empty()
      {
         return Ok((user.clone(), "".to_string()))
      }
      let encrypted_bytes = match hex::decode(encrypted_password)
      {
         Ok(bytes) => bytes,
         Err(e) =>
         {
            let errmsg = format!("Failed to hex decode encrypted password: {}", e);
            return Err(errmsg);
         }
      };
      if encrypted_bytes.is_empty()
      {
         return Ok((user.clone(), "".to_string()));
      }
      {
         let key = match Settings::get_encryption_key(false)
         {
            |  Ok(k) => k,
               Err(e) => 
               { 
                  return Err(format!("Encryption key is missing [{}]", e)); 
               }
         };
         // let mut key = match self.encryption_key.clone()
         // {
         //    |  Some(k) => k,
         //       None =>
         //       {
         //          match Settings::get_encryption_key()
         //          {
         //             |  Ok(k) => k,
         //                Err(e) => { return Err(format!("Encryption key is missing [{}]", e)); }
         //          }
         //       }
         // };
         if key.trim().is_empty() 
         {
            return Err("Encryption key is empty".to_string()); 
         }         
         match crypt::decrypt(&encrypted_bytes, &key)
         {
            |  Ok(decrypted_password) =>
               {
                  Ok((user, decrypted_password))
               }
               Err(e) =>
               {
                  let errmsg = format!("Failed to decrypt database password: {}", e);
                  eprintln!("{errmsg}");
                  Err(errmsg)
               }
         }
      }
   }

   pub fn set_database_url(&mut self, url: &str, is_local: bool) -> Result<(), String>
   //----------------------------------------------------------------
   {
      if is_local
      {
         self.local_database_url = url.to_string();
      }
      else
      {
         self.central_database_url = match url.trim().is_empty()
         {
            | true => None,
            | false => Some(url.to_string()),
         };
      }
      match self.write_settings()
      {
         | Ok(_) => Ok(()),
         | Err(e) =>
         {
            let errmsg = format!("Failed to write settings file: {}", e);
            eprintln!("{errmsg}");
            Err(errmsg)
         }
      }
   }

   pub fn set_user(&mut self, user: &str, is_local: bool) -> Result<(), String>
   //----------------------------------------------------------------
   {
      if is_local
      {
         self.local_user = match user.trim().is_empty()
         {
            | true => None,
            | false => Some(user.to_string()),
         };
      }
      else
      {
         self.central_user = match user.trim().is_empty()
         {
            | true => None,
            | false => Some(user.to_string()),
         };
      }
      match self.write_settings()
      {
         | Ok(_) => Ok(()),
         | Err(e) =>
         {
            let errmsg = format!("Failed to write settings file: {}", e);
            eprintln!("{errmsg}");
            Err(errmsg)
         }
      }
   }

   pub fn set_password(&mut self, password: &str, is_local: bool) -> Result<(), String>
   //----------------------------------------------------------------
   {
      let encrypted_password: &mut Option<String>;
      if is_local
      {
         encrypted_password = &mut self.local_encrypted_password;
      }
      else
      {
         encrypted_password = &mut self.central_encrypted_password;
      }
      if password.trim().is_empty()
      {
         *encrypted_password = None;
         match self.write_settings()
         {
            | Ok(_) => (),
            | Err(e) =>
            {
               let errmsg = format!("Failed to write settings file: {}", e);
               eprintln!("{errmsg}");
               return Err(errmsg);
            }
         }
         return Ok(());
      }
      let key = match Settings::get_encryption_key(true)
      {
         |  Ok(k) => k,
            Err(e) =>
            {
               let errmsg = format!("Encryption key is missing and failed to generate: {}", e);
               eprintln!("{errmsg}");
               return Err(errmsg);
            }
      };      
      match crypt::encrypt(password, &key)
      {
         | Ok(encrypted_data) =>
         {
            if is_local
            {
               self.local_encrypted_password = Some(hex::encode(encrypted_data));
            }
            else
            {
               self.central_encrypted_password = Some(hex::encode(encrypted_data));
            }
            match self.write_settings()
            {
               | Ok(_) => (),
               | Err(e) =>
               {
                  let errmsg = format!("Failed to write settings file: {}", e);
                  eprintln!("{errmsg}");
                  return Err(errmsg);
               }
            }
            Ok(())
         }
         | Err(e) =>
         {
            let errmsg = format!("Failed to encrypt database password: {}", e);
            eprintln!("{errmsg}");
            // self.toast_manager.error(errmsg);
            Err(errmsg)
         }
      }
   }

   pub fn set_user_password(&mut self, user: &str, password: &str, is_local: bool) -> Result<(), String>
   //----------------------------------------------------------------
   {
      let usr: &mut Option<String>;
      let encrypted_password: &mut Option<String>;
      if is_local
      {
         usr = &mut self.local_user;
         encrypted_password = &mut self.local_encrypted_password;
      }
      else
      {
         usr = &mut self.central_user;
         encrypted_password = &mut self.central_encrypted_password;
      }
      if password.trim().is_empty()
      {
         *usr = match user.trim().is_empty()
         {
            | true => None,
            | false => Some(user.to_string()),
         };
         *encrypted_password = None;
         match self.write_settings()
         {
            | Ok(_) => (),
            | Err(e) =>
            {
               let errmsg = format!("Failed to write settings file: {}", e);
               eprintln!("{errmsg}");
               return Err(errmsg);
            }
         }
         return Ok(());
      }
      let key = match Settings::get_encryption_key(true)
      {
         |  Ok(k) => k,
            Err(e) =>
            {
               let errmsg = format!("Encryption key is missing and failed to generate: {}", e);
               eprintln!("{errmsg}");
               return Err(errmsg);
            }
      };      
      match crypt::encrypt(&password, &key)
      {
         | Ok(encrypted_data) =>
         {
            *usr = match user.trim().is_empty()
            {
               | true => None,
               | false => Some(user.to_string()),
            };
            *encrypted_password = Some(hex::encode(encrypted_data));
            match self.write_settings()
            {
               | Ok(_) => (),
               | Err(e) =>
               {
                  let errmsg = format!("Failed to write settings file: {}", e);
                  eprintln!("{errmsg}");
                  return Err(errmsg);
               }
            }
            Ok(())
         }
         | Err(e) =>
         {
            let errmsg = format!("Failed to encrypt database password: {}", e);
            eprintln!("{errmsg}");
            // self.toast_manager.error(errmsg);
            Err(errmsg)
         }
      }
   }

   /// Get OS specific path to the config directory for the program
   pub fn get_config_path() -> Result<PathBuf, std::io::Error>
   //-----------------------------------------------------------------------------------------
   {
      match dirs::config_dir() // cargo add dirs
      {
         | Some(p) =>
         {
            let pp = p.join(PROGRAM);
            if !pp.exists()
            {
               match std::fs::create_dir_all(pp.as_path())
               {
                  | Ok(_) => (),
                  | Err(e) =>
                  {
                     return Err(std::io::Error::other(format!("Failed to create config directory {}: {}",
                                                            pp.display(), e)));
                  }
               }
            }
            Ok(pp)
         }
         | None =>
         {
            let mut config_path = Settings::get_home_dir();

            if env::consts::OS == "windows"
            {
               let mut pp = config_path.clone();
               pp.push("AppData/Local");
               if pp.is_dir()
               {
                  config_path.push("AppData/Local");
               }
               else
               {
                  pp.pop();
                  pp.pop();
                  pp.push("Local Settings/");
                  if pp.is_dir()
                  {
                     config_path.push("Local Settings/");
                  }
                  else
                  {
                     config_path.push("Application Data/Local Settings/");
                  }
               }
            }
            else if env::consts::OS == "macos"
            {
               config_path.push(Settings::get_home_dir());
               config_path.push(".config/");
               if ! config_path.is_dir()
               {
                  config_path.pop();
                  config_path.push("Library/");
                  config_path.push("Application Support/");
                  if ! config_path.is_dir()
                  {
                     config_path.pop();
                     config_path.pop();
                  }
               }
            }
            else
            {
               config_path.push(".config/");
            }
            config_path.push(PROGRAM);
            if config_path.exists() && !config_path.is_dir()
            {
               return Err(std::io::Error::other(format!("Config path {} exists and is not a directory",
                                                      config_path.display())));
            }
            if !config_path.exists()
            {
               std::fs::create_dir_all(config_path.as_path())?;
            }
            Ok(config_path)
         }
      }
   }

   /// Get the path to the settings file for the program.
   pub fn get_settings_path() -> Result<PathBuf, std::io::Error>
   //-------------------------------------------------------------------
   {
      let mut config_path = match Settings::get_config_path()
      {
         | Ok(p) => p,
         | Err(e) =>
         {
            eprintln!("Error getting settings path: {}", e);
            return Err(e);
         }
      };
      config_path.push("settings.json");
      Ok(config_path)
   }

   pub fn write_default_settings() -> Result<PathBuf, std::io::Error>
//-----------------------------------------------------------------------
   {
      let settings = Settings::default();
      let mut config_file = Settings::get_config_path()?;
      config_file.push("settings.json");
      let mut file = File::create(&config_file)?;
      let json = serde_json::to_string_pretty(&settings)?;
      file.write_all(json.as_bytes())?;
      // let file = File::create(&config_file)?;
      // let mut writer = BufWriter::new(file);
      // serde_json::to_writer(&mut writer, &settings)?;
      Ok(config_file)
   }

   fn read_settings(&self) -> Settings
//-----------------------------------------------------------------
   {
      let mut config_file = match Settings::get_config_path()
      {
         | Ok(p) => p,
         | Err(e) =>
         {
            eprintln!("Error getting settings path: {}", e);
            return Settings::default();
         }
      };
      config_file.push("settings.json");
      if !config_file.exists()
      {
         return Settings::default();
      }
      let file = match File::open(&config_file)
      {
         | Ok(f) => f,
         | Err(e) =>
         {
            eprintln!("Error opening settings file: {}", e);
            return Settings::default();
         }
      };
      let settings: Settings = match serde_json::from_reader(file)
      {
         | Ok(s) => s,
         | Err(e) =>
         {
            eprintln!("Error reading settings: {}", e);
            Settings::default()
         }
      };
      settings.clone()
   }

   fn get_home_fallbacks() -> PathBuf
//--------------------------------
   {
      if cfg!(target_os = "linux")
      {
         return PathBuf::from("~/");
      }
      else if cfg!(target_os = "windows")
      {
         return PathBuf::from("C:/Users/Public");
      }
      return PathBuf::from("~/");
   }

   pub fn get_home_dir() -> PathBuf
//-------------------------------
   {
      match dirs::home_dir()
      {
         | Some(h) => h,
         | None => Settings::get_home_fallbacks(),
      }
   }

   #[allow(dead_code)]
   pub fn get_home_dir_string() -> String
//-------------------------------
   {
      match dirs::home_dir()
      {
         | Some(h) => h.display().to_string(),
         | None =>
         {
            let pp = Settings::get_home_fallbacks();
            pp.display().to_string()
         }
      }
   }

   // Test helper - only available when running tests
   #[doc(hidden)]
   pub fn new_for_test(local_url: &str, central_url: &str) -> Self
   //----------------------------------------------------------------
   {
      Self {
         local_database_url: local_url.to_string(),
         local_user: None,
         local_encrypted_password: None,
         central_database_url: if central_url.is_empty() { None } else { Some(central_url.to_string()) },
         central_user: None,
         central_encrypted_password: None,
         encryption_key: None,
         last_local_update_file: None,
         last_central_update_file: None,
      }
   }
}

unsafe impl Sync for Settings {}
