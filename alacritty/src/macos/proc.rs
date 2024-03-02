use std::ffi::IntoStringError;
use std::fmt::{self, Display, Formatter};
use std::io;

/// Error during working directory retrieval.
#[derive(Debug)]
pub enum Error {
    Io(io::Error),

    /// Error converting into utf8 string.
    IntoString(IntoStringError),

    /// Expected return size didn't match libproc's.
    InvalidSize,
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::InvalidSize => None,
            Error::Io(err) => err.source(),
            Error::IntoString(err) => err.source(),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidSize => write!(f, "Invalid proc_pidinfo return size"),
            Error::Io(err) => write!(f, "Error getting current working directory: {}", err),
            Error::IntoString(err) => {
                write!(f, "Error when parsing current working directory: {}", err)
            },
        }
    }
}

impl From<io::Error> for Error {
    fn from(val: io::Error) -> Self {
        Error::Io(val)
    }
}

impl From<IntoStringError> for Error {
    fn from(val: IntoStringError) -> Self {
        Error::IntoString(val)
    }
}

/// Bindings for libproc.
#[allow(non_camel_case_types)]
mod sys {
    use std::os::raw::{c_char, c_int, c_longlong, c_void};

    type gid_t = c_int;
    type off_t = c_longlong;
    type uid_t = c_int;
    type fsid_t = fsid;

    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    pub struct fsid {
        pub val: [i32; 2usize],
    }

    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    pub struct vinfo_stat {
        pub vst_dev: u32,
        pub vst_mode: u16,
        pub vst_nlink: u16,
        pub vst_ino: u64,
        pub vst_uid: uid_t,
        pub vst_gid: gid_t,
        pub vst_atime: i64,
        pub vst_atimensec: i64,
        pub vst_mtime: i64,
        pub vst_mtimensec: i64,
        pub vst_ctime: i64,
        pub vst_ctimensec: i64,
        pub vst_birthtime: i64,
        pub vst_birthtimensec: i64,
        pub vst_size: off_t,
        pub vst_blocks: i64,
        pub vst_blksize: i32,
        pub vst_flags: u32,
        pub vst_gen: u32,
        pub vst_rdev: u32,
        pub vst_qspare: [i64; 2usize],
    }

    #[repr(C)]
    #[derive(Debug, Copy, Clone)]
    pub struct vnode_info {
        pub vi_stat: vinfo_stat,
        pub vi_type: c_int,
        pub vi_pad: c_int,
        pub vi_fsid: fsid_t,
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    pub struct vnode_info_path {
        pub vip_vi: vnode_info,
        pub vip_path: [c_char; 1024usize],
    }

    #[repr(C)]
    #[derive(Copy, Clone)]
    pub struct proc_vnodepathinfo {
        pub pvi_cdir: vnode_info_path,
        pub pvi_rdir: vnode_info_path,
    }
}
