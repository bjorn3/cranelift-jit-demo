//! Handling of everything related to debuginfo.

mod emit;
mod unwind;
mod unwind_gcc;

pub(crate) use emit::DebugRelocName;
pub(crate) use unwind::{LandingpadStrategy, UnwindContext};

#[repr(C)]
struct JitException {
    base: _Unwind_Exception,
    data: usize,
}

unsafe extern "C" fn jit_exception_cleanup(_: u64, exception: *mut _Unwind_Exception) {
    let _ = Box::from_raw(exception as *mut JitException);
}

// FIXME C-unwind
pub(crate) extern "C-unwind" fn do_throw(exception: usize) -> ! {
    unsafe {
        let res = _Unwind_RaiseException(Box::into_raw(Box::new(JitException {
            base: _Unwind_Exception {
                _exception_class: 0,
                _exception_cleanup: jit_exception_cleanup,
                _private: [0; 2],
            },
            data: exception,
        })) as *mut _Unwind_Exception);
        panic!("Failed to raise exception: {res}");
    }
}

// FIXME C-unwind
pub(crate) unsafe extern "C-unwind" fn do_resume_unwind(exception: *mut _Unwind_Exception) -> ! {
    _Unwind_Resume(exception)
}

type _Unwind_Exception_Class = u64;
type _Unwind_Word = usize;
type _Unwind_Ptr = usize;

#[link(name = "gcc_s")]
// FIXME C-unwind
extern "C" {
    fn _Unwind_RaiseException(exception: *mut _Unwind_Exception) -> u8;
    fn _Unwind_Resume(exception: *mut _Unwind_Exception) -> !;
}

extern "C" {
    fn _Unwind_DeleteException(exception: *mut _Unwind_Exception);
    fn _Unwind_GetLanguageSpecificData(ctx: *mut _Unwind_Context) -> *mut ();
    fn _Unwind_GetRegionStart(ctx: *mut _Unwind_Context) -> _Unwind_Ptr;
    fn _Unwind_GetTextRelBase(ctx: *mut _Unwind_Context) -> _Unwind_Ptr;
    fn _Unwind_GetDataRelBase(ctx: *mut _Unwind_Context) -> _Unwind_Ptr;

    fn _Unwind_GetGR(ctx: *mut _Unwind_Context, reg_index: i32) -> _Unwind_Word;
    fn _Unwind_SetGR(ctx: *mut _Unwind_Context, reg_index: i32, value: _Unwind_Word);
    fn _Unwind_GetIP(ctx: *mut _Unwind_Context) -> _Unwind_Word;
    fn _Unwind_SetIP(ctx: *mut _Unwind_Context, value: _Unwind_Word);
    fn _Unwind_GetIPInfo(ctx: *mut _Unwind_Context, ip_before_insn: *mut i32) -> _Unwind_Word;
    fn _Unwind_FindEnclosingFunction(pc: *mut ()) -> *mut ();
}

#[repr(C)]
pub struct _Unwind_Exception {
    _exception_class: u64,
    _exception_cleanup: unsafe extern "C" fn(unwind_code: u64, exception: *mut _Unwind_Exception),
    _private: [usize; 2],
}

#[repr(C)]
pub enum _Unwind_Reason_Code {
    _URC_NO_REASON = 0,
    _URC_FOREIGN_EXCEPTION_CAUGHT = 1,
    _URC_FATAL_PHASE2_ERROR = 2,
    _URC_FATAL_PHASE1_ERROR = 3,
    _URC_NORMAL_STOP = 4,
    _URC_END_OF_STACK = 5,
    _URC_HANDLER_FOUND = 6,
    _URC_INSTALL_CONTEXT = 7,
    _URC_CONTINUE_UNWIND = 8,
    _URC_FAILURE = 9, // used only by ARM EHABI
}

pub enum _Unwind_Context {}

#[repr(C)]
#[derive(Clone, Copy)]
pub enum _Unwind_Action {
    _UA_SEARCH_PHASE = 1,
    _UA_CLEANUP_PHASE = 2,
    _UA_HANDLER_FRAME = 4,
    _UA_FORCE_UNWIND = 8,
    _UA_END_OF_STACK = 16,
}
