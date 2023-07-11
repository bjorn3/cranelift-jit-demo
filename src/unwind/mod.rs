//! Handling of everything related to debuginfo.

mod emit;
mod unwind;
mod unwind_custom;
mod unwind_fast;
mod unwind_gcc;

use cranelift::codegen::ir::Value;
use cranelift::codegen::Context;
use cranelift::prelude::FunctionBuilder;
use cranelift_jit::JITModule;
use cranelift_module::FuncId;
pub use unwind::EhFrameUnwinder;
pub use unwind_custom::CustomUnwinder;

// FIXME add non-eh_frame based unwinder option

pub unsafe trait Unwinder {
    fn register_function(&mut self, module: &mut JITModule, func_id: FuncId, context: &Context);

    unsafe fn call_and_catch_unwind0(
        &self,
        func: extern "C-unwind" fn() -> usize,
    ) -> Result<usize, usize>;
    unsafe fn call_and_catch_unwind1(
        &self,
        func: extern "C-unwind" fn(usize) -> usize,
        arg: usize,
    ) -> Result<usize, usize>;
    unsafe fn call_and_catch_unwind2(
        &self,
        func: extern "C-unwind" fn(usize, usize) -> usize,
        arg0: usize,
        arg1: usize,
    ) -> Result<usize, usize>;

    fn get_exception_data(&self, builder: &mut FunctionBuilder, exception_val: Value) -> Value;
    fn throw_func(&self) -> unsafe extern "C-unwind" fn(exception: usize) -> !;
    fn resume_unwind_func(
        &self,
    ) -> unsafe extern "C-unwind" fn(exception: *mut _Unwind_Exception) -> !;
}

// UNWIND_DATA_REG definitions copied from rust's personality function definition
#[cfg(target_arch = "x86")]
const UNWIND_DATA_REG: (i32, i32) = (0, 2); // EAX, EDX

#[cfg(target_arch = "x86_64")]
const UNWIND_DATA_REG: (i32, i32) = (0, 1); // RAX, RDX

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
const UNWIND_DATA_REG: (i32, i32) = (0, 1); // R0, R1 / X0, X1

#[cfg(target_arch = "m68k")]
const UNWIND_DATA_REG: (i32, i32) = (0, 1); // D0, D1

#[cfg(any(target_arch = "mips", target_arch = "mips64"))]
const UNWIND_DATA_REG: (i32, i32) = (4, 5); // A0, A1

#[cfg(any(target_arch = "powerpc", target_arch = "powerpc64"))]
const UNWIND_DATA_REG: (i32, i32) = (3, 4); // R3, R4 / X3, X4

#[cfg(target_arch = "s390x")]
const UNWIND_DATA_REG: (i32, i32) = (6, 7); // R6, R7

#[cfg(any(target_arch = "sparc", target_arch = "sparc64"))]
const UNWIND_DATA_REG: (i32, i32) = (24, 25); // I0, I1

#[cfg(target_arch = "hexagon")]
const UNWIND_DATA_REG: (i32, i32) = (0, 1); // R0, R1

#[cfg(any(target_arch = "riscv64", target_arch = "riscv32"))]
const UNWIND_DATA_REG: (i32, i32) = (10, 11); // x10, x11

#[cfg(target_arch = "loongarch64")]
const UNWIND_DATA_REG: (i32, i32) = (4, 5); // a0, a1

#[allow(non_camel_case_types)]
type _Unwind_Exception_Class = u64;
#[allow(non_camel_case_types)]
type _Unwind_Word = usize;
#[allow(non_camel_case_types)]
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

#[allow(non_camel_case_types)]
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

#[allow(non_camel_case_types)]
pub enum _Unwind_Context {}

#[allow(non_camel_case_types)]
#[repr(C)]
#[derive(Clone, Copy)]
pub enum _Unwind_Action {
    _UA_SEARCH_PHASE = 1,
    _UA_CLEANUP_PHASE = 2,
    _UA_HANDLER_FRAME = 4,
    _UA_FORCE_UNWIND = 8,
    _UA_END_OF_STACK = 16,
}
