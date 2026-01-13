use std::io::{self, Write};

use clap::{Parser, Subcommand};
use colored::Colorize;

use dejacmd::crypt;

#[derive(Parser)]
#[command(name = "dejacmd-passwd")]
#[command(about = "Password encryption and key generation utility")]
#[command(long_about =
r#"Utility for generating encryption keys and encrypting passwords.
This tool uses AES-256-GCM encryption to securely encrypt passwords.

Example workflow:
1. Generate a key: dejacmd-passwd genkey
2. Encrypt a password: dejacmd-passwd encrypt -k <key>
3. Use the encrypted password in your database configuration"#)]
#[command(after_help =
r#"Command Aliases:
genkey = g or ge or gen or genk
encrypt = e or en or enc or encr"#)]
struct Cli
{
   #[command(subcommand)]
   command: Commands,
}

#[derive(Subcommand)]
enum Commands
{
   #[command(about = "Generate a new encryption key")]
   #[command(aliases = ["g", "ge", "gen", "genk"])]
   Genkey,

   #[command(about = "Encrypt a password")]
   #[command(aliases = ["e", "en", "enc", "encr"])]
   Encrypt
   {
      #[arg(short = 'k', long = "key", help = "Encryption key (hex format). If not provided, will prompt for it.")]
      key: Option<String>,

      #[arg(short = 'p', long = "password", help = "Password to encrypt. If not provided, will prompt for it.")]
      password: Option<String>,

      #[arg(short = 's', long = "show", help = "Show password when entering from console")]
      is_show_password: bool,
   },
}

fn main()
{
   let args = Cli::parse();

   match args.command
   {
      Commands::Genkey =>
      {
         let key = crypt::generate_key();
         println!("{}", "Generated encryption key:".bright_green());
         println!("{}", key.bright_white());
         println!();
         println!("{}", "Save this key securely! You'll need it to encrypt passwords.".bright_yellow());
      },

      Commands::Encrypt { key, password, is_show_password } =>
      {
         // Get the encryption key
         let encryption_key = if let Some(k) = key
         {
            k
         }
         else
         {
            print!("{}", "Enter encryption key: ".bright_cyan());
            io::stdout().flush().unwrap();
            let mut input = String::new();
            io::stdin().read_line(&mut input).expect("Failed to read key");
            input.trim().to_string()
         };

         if encryption_key.is_empty()
         {
            eprintln!("{}", "Error: Encryption key is required".bright_red());
            std::process::exit(1);
         }

         // Validate key format (should be hex)
         if encryption_key.len() != 64 || !encryption_key.chars().all(|c| c.is_ascii_hexdigit())
         {
            eprintln!("{}", "Error: Invalid encryption key format. Key must be 64 hex characters.".bright_red());
            eprintln!("{}", "Generate a new key with: dejacmd-passwd genkey".bright_yellow());
            std::process::exit(1);
         }

         // Get the password to encrypt
         let pwd = if let Some(p) = password
         {
            p
         }
         else
         {
            prompt_for_password(is_show_password)
         };

         if pwd.is_empty()
         {
            eprintln!("{}", "Error: Password cannot be empty".bright_red());
            std::process::exit(1);
         }

         // Encrypt the password
         match crypt::encrypt(&pwd, &encryption_key)
         {
            Ok(encrypted_data) =>
            {
               let encrypted_hex = hex::encode(encrypted_data);
               println!("{}", "Encrypted password:".bright_green());
               println!("{}", encrypted_hex.bright_white());
               println!();
               println!("{}", "Use this encrypted password in your database configuration.".bright_cyan());
            },
            Err(e) =>
            {
               eprintln!("{}: {}", "Error encrypting password".bright_red(), e);
               std::process::exit(1);
            }
         }
      },
   }
}

fn prompt_for_password(show_password: bool) -> String
{
   print!("Enter password: ");
   io::stdout().flush().unwrap();

   if show_password
   {
      // Read password with echo enabled
      let mut password = String::new();
      io::stdin().read_line(&mut password).expect("Failed to read password");
      password.trim().to_string()
   }
   else
   {
      // Read password without echo using rpassword crate
      match rpassword::read_password()
      {
         Ok(pwd) => pwd,
         Err(e) =>
         {
            eprintln!("{}: {}", "Error reading password".bright_red(), e);
            String::new()
         }
      }
   }
}
