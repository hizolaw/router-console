//! Privileged, bounded Unix-socket PAM adapter. Never logs request contents.
use clap::Parser;
use serde::Deserialize;
use std::{
    ffi::{CStr, CString, c_char, c_int, c_void},
    io::{BufRead, BufReader, Write},
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::PathBuf,
    time::Duration,
};
#[derive(Parser)]
struct Args {
    #[arg(long, default_value = "/run/router-console-auth/auth.sock")]
    socket: PathBuf,
    #[arg(long)]
    pam_library: PathBuf,
    #[arg(long, default_value = "root")]
    allowed_users: String,
    #[arg(long, default_value = "router-console")]
    pam_service: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    username: String,
    password: String,
}
#[repr(C)]
struct Message {
    style: c_int,
    msg: *const c_char,
}
#[repr(C)]
struct Response {
    response: *mut c_char,
    retcode: c_int,
}
#[repr(C)]
struct Conversation {
    callback: unsafe extern "C" fn(
        c_int,
        *const *const Message,
        *mut *mut Response,
        *mut c_void,
    ) -> c_int,
    data: *mut c_void,
}
struct Credentials {
    user: CString,
    password: CString,
}
unsafe extern "C" fn converse(
    n: c_int,
    messages: *const *const Message,
    out: *mut *mut Response,
    data: *mut c_void,
) -> c_int {
    if !(1..=16).contains(&n) || messages.is_null() || out.is_null() || data.is_null() {
        return 19;
    }
    unsafe {
        let creds = &*(data as *const Credentials);
        let responses = libc::calloc(n as usize, std::mem::size_of::<Response>()) as *mut Response;
        if responses.is_null() {
            return 19;
        }
        for i in 0..n as usize {
            let message = *messages.add(i);
            let value = if message.is_null() {
                None
            } else {
                match (*message).style {
                    1 => Some(creds.password.as_ptr()),
                    2 => Some(creds.user.as_ptr()),
                    3 | 4 => Some(std::ptr::null()),
                    _ => None,
                }
            };
            if let Some(value) = value {
                if !value.is_null() {
                    (*responses.add(i)).response = libc::strdup(value);
                    if (*responses.add(i)).response.is_null() {
                        for j in 0..i {
                            libc::free((*responses.add(j)).response.cast());
                        }
                        libc::free(responses.cast());
                        return 19;
                    }
                }
            } else {
                for j in 0..i {
                    libc::free((*responses.add(j)).response.cast());
                }
                libc::free(responses.cast());
                return 19;
            }
        }
        *out = responses;
        0
    }
}
fn valid(r: &Request, allowed: &[&str]) -> bool {
    !r.username.is_empty()
        && r.username.len() <= 64
        && r.username
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))
        && allowed.contains(&r.username.as_str())
        && !r.password.is_empty()
        && r.password.len() <= 1024
        && !r.password.contains('\0')
}
struct Pam {
    library: *mut c_void,
}
impl Drop for Pam {
    fn drop(&mut self) {
        unsafe {
            libc::dlclose(self.library);
        }
    }
}
impl Pam {
    fn load(path: &std::path::Path) -> Result<Self, ()> {
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(path.as_os_str().as_bytes()).map_err(|_| ())?;
        let library = unsafe { libc::dlopen(path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if library.is_null() {
            Err(())
        } else {
            Ok(Self { library })
        }
    }
    unsafe fn symbol(&self, name: &CStr) -> *mut c_void {
        unsafe { libc::dlsym(self.library, name.as_ptr()) }
    }
    fn authenticate(&self, r: &Request, service: &str) -> bool {
        unsafe {
            let start = self.symbol(c"pam_start");
            let auth = self.symbol(c"pam_authenticate");
            let account = self.symbol(c"pam_acct_mgmt");
            let end = self.symbol(c"pam_end");
            if [start, auth, account, end].iter().any(|p| p.is_null()) {
                return false;
            }
            let start: unsafe extern "C" fn(
                *const c_char,
                *const c_char,
                *const Conversation,
                *mut *mut c_void,
            ) -> c_int = std::mem::transmute(start);
            let auth: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = std::mem::transmute(auth);
            let account: unsafe extern "C" fn(*mut c_void, c_int) -> c_int =
                std::mem::transmute(account);
            let end: unsafe extern "C" fn(*mut c_void, c_int) -> c_int = std::mem::transmute(end);
            let Ok(user) = CString::new(r.username.as_str()) else {
                return false;
            };
            let Ok(password) = CString::new(r.password.as_str()) else {
                return false;
            };
            let Ok(service) = CString::new(service) else {
                return false;
            };
            let mut credentials = Credentials { user, password };
            let conversation = Conversation {
                callback: converse,
                data: (&mut credentials as *mut Credentials).cast(),
            };
            let mut handle = std::ptr::null_mut();
            let mut status = start(
                service.as_ptr(),
                credentials.user.as_ptr(),
                &conversation,
                &mut handle,
            );
            if status != 0 {
                return false;
            }
            status = auth(handle, 1); // PAM_DISALLOW_NULL_AUTHTOK
            if status == 0 {
                status = account(handle, 1);
            }
            end(handle, status);
            status == 0
        }
    }
}
fn main() {
    if run().is_err() {
        eprintln!("router-console-auth: initialization failed");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if unsafe { libc::geteuid() } != 0 {
        return Err("root required".into());
    }
    let pam = Pam::load(&args.pam_library).map_err(|_| "cannot load PAM")?;
    let allowed: Vec<_> = args.allowed_users.split(',').collect();
    // The parent directory is private to root and the console-auth group.
    if args.socket.exists() {
        std::fs::remove_file(&args.socket)?;
    }
    let listener = UnixListener::bind(&args.socket)?;
    std::fs::set_permissions(&args.socket, std::fs::Permissions::from_mode(0o660))?;
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
        let mut data = Vec::new();
        let mut reader = BufReader::new((&stream).take(4097));
        let ok = reader.read_until(b'\n', &mut data).is_ok()
            && data.len() <= 4096
            && data.last() == Some(&b'\n')
            && serde_json::from_slice::<Request>(&data)
                .is_ok_and(|r| valid(&r, &allowed) && pam.authenticate(&r, &args.pam_service));
        let _ = stream.write_all(if ok {
            b"{\"ok\":true}\n"
        } else {
            b"{\"ok\":false}\n"
        });
    }
    Ok(())
}
use std::io::Read;
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_empty_and_unlisted() {
        for (user, password) in [("root", ""), ("nobody", "secret"), ("root\0", "secret")] {
            assert!(!valid(
                &Request {
                    username: user.into(),
                    password: password.into()
                },
                &["root"]
            ));
        }
        assert!(valid(
            &Request {
                username: "root".into(),
                password: "secret".into()
            },
            &["root"]
        ));
    }
    #[test]
    fn strict_protocol() {
        assert!(
            serde_json::from_str::<Request>(
                r#"{"username":"root","password":"x","command":"oops"}"#
            )
            .is_err()
        );
    }
}
