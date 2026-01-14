use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use crossbeam::atomic::AtomicCell;

use regex::Regex;
use clap::Parser;
use colored::Colorize;
use short_uuid::ShortUuid;
use include_dir::{include_dir, Dir};

use dejacmd::settings::Settings;
use dejacmd::{CREATE_INDEX_SQL, CREATE_TABLE_SQL, INSERT_HISTORY_SQL, connections, fix_placeholders, get_database};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args
{
   history: String,

   #[arg(short = 's' ,long = "status", default_value_t = -1,
         help = "Exit status of invoked command")]
   pub status: i64,

   #[arg(short = 'p' ,long = "pid", default_value_t = -1,
         help = "Process ID of invoked command")]
   pub pid: i64,

   #[arg(short = 'l' ,long = "log", default_value = "stderr",
         help = r#"Log errors (path to file or "stderr" or "stdout")"#)]
   pub log_destination: String,
}

pub(crate) static ASSETS_DIR: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/assets");

async fn apply_database_updates(log_destination: &str)
//----------------------------------------------------------------------------------------------------------------------
{
   let settings_file = match Settings::get_settings_path()
   {
      Ok(p) => p.display().to_string(),
      Err(_e) => "".to_string()
   };
   let mut settings = match Settings::new().get_settings()
   {
      Ok(s) => s,
      Err(e) =>
      {
         log(&log_destination,
            format!("{} {} [{}] - {}", "Error loading settings file ", settings_file, e,
               "Creating/using default settings with SQLite database."));
         _ = Settings::write_default_settings();
         Settings::default()
      }
   };
   let last_local_update = settings.last_local_update_file.clone().unwrap_or_else(|| "0000000.sql".to_string());
   let last_central_update = settings.last_central_update_file.clone().unwrap_or_else(|| "0000000.sql".to_string());

   // Collect and sort SQL update files
   let mut sql_files: Vec<_> = ASSETS_DIR.files()
      .filter(|file| {
         let path_str = file.path().to_string_lossy();
         path_str.ends_with(".sql") &&
         path_str.chars().take(7).all(|c| c.is_ascii_digit() || c == '/')
      })
      .collect();

   sql_files.sort_by_key(|file| {
      file.path().file_name().and_then(|n| n.to_str()).unwrap_or("")
   });
   let last_file = sql_files.last()
      .and_then(|file| file.path().file_name().and_then(|n| n.to_str()))
      .unwrap_or("");
   if last_file <= last_local_update.as_str() && last_file <= last_central_update.as_str()
   {
      return;
   }

   let new_last_local_update: AtomicCell<String> = AtomicCell::new("".to_string());
   let new_last_central_update: AtomicCell<String> = AtomicCell::new("".to_string());
   let (local_pool_opt, local_scheme, central_pool_opt, central_scheme) = match connections(&settings, false, false).await
   {
      Ok(c) => c,
      Err(e) => 
      {
         log(log_destination, format!("apply_database_updates: Error connecting to database(s): {}", e));
         return;
      }
   };

   // Execute updates after the last one
   for file in sql_files 
   {
      let filename = file.path().file_name()
         .and_then(|n| n.to_str())
         .unwrap_or("");

      // Read and execute the SQL script
      let mut local_error_messages: Vec<String> = vec![];
      let mut central_error_messages: Vec<String> = vec![];
      if let Some(sql_content) = file.contents_utf8() 
      {
         let local_queries = async
         //==========================================================
         {
            if local_pool_opt.is_none()
            {
               return Ok(sqlx::any::AnyQueryResult::default());
            }
            if filename <= last_local_update.as_str()
            {
               return Ok(sqlx::any::AnyQueryResult::default());
            }

            let pool = local_pool_opt.as_ref().unwrap();
            let sql = dejacmd::fix_placeholders(sql_content, &local_scheme);
            let result =  sqlx::query(&sql).execute(pool).await;
            if result.is_err()
            {
               local_error_messages.push(format!("Failed to execute update {}: {}", filename, result.as_ref().err().unwrap().to_string()));
            }
            else
            {
               new_last_local_update.store(filename.to_string());
            }         
            result   
         };
         let central_queries = async
         //============================================================
         {
            if central_pool_opt.is_none()
            {
               return Ok(sqlx::any::AnyQueryResult::default());
            }
            if filename <= last_central_update.as_str()
            {
               return Ok(sqlx::any::AnyQueryResult::default());
            }
            let pool = central_pool_opt.as_ref().unwrap();
            let sql = dejacmd::fix_placeholders(sql_content, &central_scheme);
            let result =  sqlx::query(&sql).execute(pool).await;
            if result.is_err()
            {
               central_error_messages.push(format!("Failed to execute update {}: {}", filename, result.as_ref().err().unwrap().to_string()));
            }
            else
            {
               new_last_central_update.store(filename.to_string());
            }         
            result   
         };

         let (local_result, central_result) = tokio::join!(local_queries, central_queries);
         if local_result.is_err() 
         {
            for msg in &local_error_messages
            {
               log(log_destination, format!("Local apply_database_updates: {}", msg));
            }
         }
         else
         {
            let final_update = new_last_local_update.take();
            if !final_update.is_empty() && final_update != last_local_update
            {
               settings.last_local_update_file = Some(final_update.clone());
               match settings.write_settings()
               {
                  Ok(_) => {},
                  Err(e) => log(log_destination, format!("Error saving updated last_update_file '{}' to settings: {}", final_update, e)),
               }
            }
         }
         if central_result.is_err()
         {
            for msg in &central_error_messages
            {
               log(log_destination, format!("Central apply_database_updates: {}", msg));
            }
         }         
         else
         {
            let final_update = new_last_central_update.take();
            if !final_update.is_empty() && final_update != last_central_update
            {
               settings.last_central_update_file = Some(final_update.clone());
               match settings.write_settings()
               {
                  Ok(_) => {},
                  Err(e) => log(log_destination, format!("Error saving updated last_update_file '{}' to settings: {}", final_update, e)),
               }
            }
         }
      }
   }
   
}

#[tokio::main]
async fn main() -> std::process::ExitCode
//----------------------------------
{
   let args = Args::parse();

   sqlx::any::install_default_drivers(); // According to sqlx/src/any/install_drivers_note.md to prevent panic
   apply_database_updates(&args.log_destination).await;

   // 66774  2026-01-13 17:45:51 ls -ltrh 
   let re = Regex::new(r"^\s*(\d+)\s+(\d{4}-\d{2}-\d{2}\s+\d{2}:\d{2}:\d{2})\s+(.+)$").unwrap();

   let text = args.history;
   let command_date: String;
   let command: String;
   if let Some(capture) = re.captures(&text)
   {
      // let num = &capture[1];
      command_date = capture[2].to_string();
      command = capture[3].to_string();
   }
   else
   {
      log(&args.log_destination, format!("{} '{}'", "Failed to parse history line:", &text));
      return std::process::ExitCode::from(1);
   }
   let ip = match localip::get_local_ip()
   {
      Ok(i) => i.to_string(),
      Err(_) => "".to_string()
   };

   let settings_file = match Settings::get_settings_path()
   {
      Ok(p) => p.display().to_string(),
      Err(_e) => "".to_string()
   };
   let settings = match Settings::new().get_settings()
   {
      Ok(s) => s,
      Err(e) =>
      {
         log(&args.log_destination,
            format!("{} {} [{}] - {}", "Error loading settings file ", settings_file, e,
               "Creating/using default settings with SQLite database."));
         _ = Settings::write_default_settings();
         Settings::default()
      }
   };

   // println!("local database URL: {}", settings.get_local_database_url().yellow());
   
   let (shell, os_user_id, os_user, cwd) = get_process_info().await;
   let id = ShortUuid::generate();
   let mut local_error_messages: Vec<String> = vec![];
   let mut central_error_messages: Vec<String> = vec![];
   let mut local_location = 0;
   let mut central_location = 0;
   let os = std::env::consts::OS.to_string();
   let local_queries = async
   {
      let url = settings.get_local_database_url();
      if url.trim().is_empty()
      {
         return Ok(sqlx::any::AnyQueryResult::default());
      }
      let (user, password) = match settings.get_credentials(true)
      {
         Ok((u, p)) => (u, p),
         Err(_e) => ("".to_string(), "".to_string())
      };
      local_location = 1;
      let (local_pool, local_scheme) = match get_database(&url, &user, &password).await
      {
         Ok((pool, scheme)) => (pool, scheme),
         Err(e) =>
         {
            let errmsg = format!("{} {}", "Error connecting to local database:", e);
            local_error_messages.push(errmsg);
            return Ok(sqlx::any::AnyQueryResult::default());
         }
      };
      if let Some(pool) = local_pool.as_ref()
      {
         local_location = 2;
         let mut result =  sqlx::query( CREATE_TABLE_SQL ).execute(pool).await;
         if result.is_err()
         {
            local_error_messages.push(format!("{} {}", "Error creating table in local database:", result.as_ref().err().unwrap().to_string()));
            return result;
         }
         local_location = 3;
         result = sqlx::query( CREATE_INDEX_SQL ).execute(pool).await;
         if result.is_err()
         {
            local_error_messages.push(format!("{} {}", "Error creating index in local database:", result.as_ref().err().unwrap().to_string()));
            return result;
         }
         local_location = 4;
         let sql = fix_placeholders(INSERT_HISTORY_SQL, &local_scheme);
         result = sqlx::query( &sql )
         .bind(id.to_string())
         .bind(&command_date)
         .bind( cwd.display().to_string() )
         .bind( shell.clone() )
         .bind( os_user_id ) //os_user_id!= -1 { Some(user_id) } else { None } )
         .bind( os_user.clone() )
         .bind( ip.clone() )
         .bind( os.clone() )
         .bind( args.status )
         .bind( command.clone() )
         .execute(pool).await;
         if result.is_err()
         {
            let values = format!("VALUES ( {}, {}, {}, {}, {}, {}, {}, {}, {} )",
               id, command_date.clone(), cwd.display(), shell.clone(), os_user_id, os_user.clone(),
               ip.clone(), args.status, command.clone() );
            local_error_messages.push(format!("{}: {} {}", "Error inserting command into local database:", sql, values));
         }
         result
      }
      else
      {
         Ok(sqlx::any::AnyQueryResult::default())
      }
   };
   let central_queries = async
   {
      let url = settings.get_central_database_url();
      if url.trim().is_empty()
      {
         return Ok(sqlx::any::AnyQueryResult::default());
      }
      let (user, password) = match settings.get_credentials(false)
      {
         Ok((u, p)) => (u, p),
         Err(_e) => ("".to_string(), "".to_string())
      };
      central_location = 1;
      let (central_pool, central_scheme) = match get_database(&url, &user, &password).await
      {
         Ok((pool, scheme)) => (pool, scheme),
         Err(e) =>
         {
            let errmsg = format!("{} {}", "Error connecting to central database:", e);
            central_error_messages.push(errmsg);
            return Ok(sqlx::any::AnyQueryResult::default());
         }
      };

      central_location = 2;
      if let Some(pool) = central_pool.as_ref()
      {
         let mut result =  sqlx::query( CREATE_TABLE_SQL ).execute(pool).await;
         if result.is_err()
         {
            central_error_messages.push(format!("{} {}", "Error creating table in central database:", result.as_ref().err().unwrap().to_string()));
            return result;
         }
         central_location = 3;
         result = sqlx::query( CREATE_INDEX_SQL ).execute(pool).await;
         if result.is_err()
         {
            central_error_messages.push(format!("{} {}", "Error creating index in central database:", result.as_ref().err().unwrap().to_string()));
            return result;
         }
         central_location = 4;
         let sql = fix_placeholders(INSERT_HISTORY_SQL, &central_scheme);
         result = sqlx::query( &sql )
         .bind(id.to_string())
         .bind(&command_date)
         .bind( cwd.display().to_string() )
         .bind( shell.clone() )
         .bind( os_user_id ) //os_user_id!= -1 { Some(user_id) } else { None } )
         .bind( os_user.clone() )
         .bind( ip.clone() )
         .bind( os.clone() )
         .bind( args.status )
         .bind( command.clone() )
         .execute(pool).await;
         if result.is_err()
         {
            let values = format!("VALUES ( {}, {}, {}, {}, {}, {}, {}, {}, {} )",
               id, command_date.clone(), cwd.display(), shell.clone(), os_user_id, os_user.clone(),
               ip.clone(), args.status, command.clone() );
            central_error_messages.push(format!("{}: {} {}", "Error inserting command into central database:", sql, values));
         }
         result
      }
      else
      {
         Ok(sqlx::any::AnyQueryResult::default())
      }
   };

   let (local_result, central_result) = tokio::join!(local_queries, central_queries);

   let mut status = 0;
   if local_result.is_err()
   {
      log(&args.log_destination, 
         format!("{} ({}) {}", "Error inserting command into local history database:", local_location, local_result.err().unwrap().to_string()));
      status |= 1;
   }
   if central_result.is_err()
   {
      log(&args.log_destination, 
         format!("{} ({}) {}", "Error inserting command into central history database:", central_location, central_result.err().unwrap().to_string()));
      status |= 2;
   }
   if local_error_messages.len() > 0
   {
      for msg in local_error_messages
      {
         log(&args.log_destination, format!("{}", msg));
      }
   }
   if central_error_messages.len() > 0
   {
      for msg in central_error_messages
      {
         log(&args.log_destination, format!("{}", msg.red()));
      }
   }
   std::process::ExitCode::from(status)
}


fn log(destination: &str, message: String)
//------------------------------------------------
{
   if destination.to_lowercase() == "stderr"
   {
      eprintln!("{}", message);
   }
   else if destination.to_lowercase() == "stdout"
   {
      println!("{}", message);
   }
   else
   {      
      let log_path = Path::new(&destination);
      let mut file = match OpenOptions::new()
         .create(true)
         .append(true)
         .open(log_path)
      {
         Ok(f) => f,
         Err(e) =>
         {
            eprintln!("{} {} [{}]", "Error opening log file:".red(), destination.red(), e.to_string().bright_red());
            eprintln!("Log message was: {}", message);
            return;
         }
      };
      if let Err(e) = writeln!(file, "{}", message)
      {
         eprintln!("{} {} [{}]", "Error writing to log file:".red(), destination.red(), e.to_string().bright_red());
         eprintln!("Log message was: {}", message);
      }
   }
}

#[allow(unused)]
async fn get_process_info() -> (String, i32, String, PathBuf)
//------------------------------------------------------------------------------------------------------
{
   let _my_pid = std::process::id();
   let mut shell: String = "".to_string();
   let _my_parent_pid: i32 = -1;
   let mut user_id: i32 = -1;
   let mut cwd: PathBuf = PathBuf::new();
   let mut user: String = std::env::var("USER").unwrap_or("".to_string());

   #[cfg(target_os = "linux")]
   {
      if let Ok(p) = procfs::process::Process::myself() {
         cwd = p.cwd().unwrap_or(std::path::PathBuf::new());
         user_id = match p.uid()
         {
            Ok(uid) => uid as i32,
            Err(_) => -1
         };
         if let Ok(uid) = p.loginuid() {
            user_id = uid as i32;
         }
         let (sh, sh_cwd) = find_linux_shell(&p);
         // println!("Found shell: {} at {}", sh, sh_cwd.display());
         shell = sh;
         if !sh_cwd.as_os_str().is_empty() { cwd = sh_cwd; }
      };
      if shell.is_empty()
      {
         shell = std::env::var("SHELL").unwrap_or("".to_string());
      } // or // "/proc/$$/comm"
   }

   #[cfg(any(target_os = "macos", target_os = "freebsd"))]
   {
      use nix::unistd::{getcwd, getuid, User, Uid};
      cwd = match getcwd()
      {
         Ok(p) => p,
         Err(_) => Settings::get_home_dir()
      };
      let uid: Uid = getuid();
      user_id = uid.as_raw() as i32;
      if let Ok(user_info) = User::from_uid(uid) && let Some(u) = user_info
      {
         user = u.name;
      }
      shell = std::env::var("SHELL").unwrap_or("".to_string());
   }

   #[cfg(target_os = "windows")]
   {
      shell = std::env::var("COMSPEC").unwrap_or("".to_string());
      user_id = -1;
      user = std::env::var("USERNAME").unwrap_or("".to_string());
      cwd = match std::env::current_dir()
      {
         Ok(p) => p,
         Err(_) => Settings::get_home_dir()
      };
   }

   let shell_buf = PathBuf::from(&shell);
   shell = match shell_buf.file_name()
   {
      Some(s) => s.to_string_lossy().to_string().replacen("-", "", 2).to_string().trim().to_string(),
      None => shell
   };
   (shell, user_id, user, cwd)
}

#[cfg(target_os = "linux")]
fn find_linux_shell(proc: &procfs::process::Process) -> (String, PathBuf)
//--------------------------------------------------------------
{
   let mut shell = "".to_string();
   let mut cwd = PathBuf::new();
   let mut ppid: i32;
   ppid = match proc.stat()
   {
      Ok(ms) => ms.ppid,
      Err(_) => -1
   };
   while ppid > 0
   {
      let process = match procfs::process::Process::new(ppid)
      {
         Ok(pp) => pp,
         Err(_) =>
         {
            shell = "".to_string();
            break;
         }
      };
      let cmdline = process.cmdline().unwrap_or(vec![]);
      if ! cmdline.is_empty()
      {
         let cmd = &cmdline[0];
         if cmd.contains("bash") || cmd.contains("zsh") || cmd.contains("pwsh") || cmd.contains("fish") //|| cmd.contains("ksh") || cmd.contains("tcsh") || cmd.contains("csh") || cmd.contains("sh")
         {
            shell = cmd.to_string();
            cwd = process.cwd().unwrap_or(std::path::PathBuf::new());
            break;
         }
      }
      ppid = match process.stat()
      {
         Ok(ms) => ms.ppid,
         Err(_) => -1
      };
   }
   (shell, cwd)
}
