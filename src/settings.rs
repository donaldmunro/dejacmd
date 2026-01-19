//#![feature(os_str_display)]
use std::{env, fs::File, io::Write, path::PathBuf};

use aes_gcm::{ // cargo add aes-gcm
    aead::{KeyInit, OsRng},
    Aes256Gcm
};

use crate::crypt;

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

   pub fn get_settings_or_default(&self) -> Settings
   //-------------------------------------------
   {
      match self.get_settings()
      {
         | Ok(s) => s,
         | Err(_) => Settings::default(),
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
         let key = match self.encryption_key.clone()
         {
            |  Some(k) => k,
               None => { return Err("Encryption key is missing".to_string()); }
         };
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
      let key = match &self.encryption_key
      {
         |  Some(k) => k.clone(),
            None =>
            {
               let new_key = Aes256Gcm::generate_key(&mut OsRng);
               self.encryption_key = Some(hex::encode(new_key));
               self.encryption_key.clone().unwrap()
            }
      };
      match crypt::encrypt(&password, &key)
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
      let key = match self.encryption_key.clone()
      {
         |  Some(k) => k,
            None =>
            {
               let new_key = Aes256Gcm::generate_key(&mut OsRng);
               self.encryption_key = Some(hex::encode(new_key));
               self.encryption_key.clone().unwrap()
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
