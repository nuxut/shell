//! libpam'in kullandığımız küçük alt kümesi (Linux-PAM, `<security/pam_appl.h>`).

#![allow(non_camel_case_types)]

use libc::{c_char, c_int, c_void};

/// Opak `pam_handle_t`.
#[repr(C)]
pub struct pam_handle_t {
    _private: [u8; 0],
}

#[repr(C)]
pub struct pam_message {
    pub msg_style: c_int,
    pub msg: *const c_char,
}

#[repr(C)]
pub struct pam_response {
    pub resp: *mut c_char,
    pub resp_retcode: c_int,
}

/// Linux-PAM `msg`'i `num_msg` adet işaretçiden oluşan dizi olarak verir.
pub type ConvFn = unsafe extern "C" fn(
    num_msg: c_int,
    msg: *mut *const pam_message,
    resp: *mut *mut pam_response,
    appdata_ptr: *mut c_void,
) -> c_int;

#[repr(C)]
pub struct pam_conv {
    pub conv: Option<ConvFn>,
    pub appdata_ptr: *mut c_void,
}

pub const PAM_SUCCESS: c_int = 0;
pub const PAM_SYSTEM_ERR: c_int = 4;
pub const PAM_BUF_ERR: c_int = 5;
pub const PAM_PERM_DENIED: c_int = 6;
pub const PAM_AUTH_ERR: c_int = 7;
pub const PAM_CRED_INSUFFICIENT: c_int = 8;
pub const PAM_AUTHINFO_UNAVAIL: c_int = 9;
pub const PAM_USER_UNKNOWN: c_int = 10;
pub const PAM_MAXTRIES: c_int = 11;
pub const PAM_CONV_ERR: c_int = 19;

pub const PAM_REFRESH_CRED: c_int = 0x0010;

pub const PAM_PROMPT_ECHO_OFF: c_int = 1;
pub const PAM_PROMPT_ECHO_ON: c_int = 2;
pub const PAM_ERROR_MSG: c_int = 3;
pub const PAM_TEXT_INFO: c_int = 4;

pub const PAM_MAX_NUM_MSG: c_int = 32;
pub const PAM_MAX_MSG_SIZE: usize = 512;

#[link(name = "pam")]
unsafe extern "C" {
    pub fn pam_start(
        service_name: *const c_char,
        user: *const c_char,
        pam_conversation: *const pam_conv,
        pamh: *mut *mut pam_handle_t,
    ) -> c_int;
    #[cfg(feature = "testing")]
    pub fn pam_start_confdir(
        service_name: *const c_char,
        user: *const c_char,
        pam_conversation: *const pam_conv,
        confdir: *const c_char,
        pamh: *mut *mut pam_handle_t,
    ) -> c_int;
    pub fn pam_authenticate(pamh: *mut pam_handle_t, flags: c_int) -> c_int;
    pub fn pam_setcred(pamh: *mut pam_handle_t, flags: c_int) -> c_int;
    pub fn pam_end(pamh: *mut pam_handle_t, pam_status: c_int) -> c_int;
}
