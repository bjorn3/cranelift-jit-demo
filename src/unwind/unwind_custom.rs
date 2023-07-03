use std::arch::asm;
use std::collections::HashMap;

use cranelift::prelude::{FunctionBuilder, Value};

use crate::unwind::Unwinder;

#[no_mangle]
static mut CURRENT_CALL_AND_UNWIND_RET_ADDR: usize = 0;

#[no_mangle]
static mut EXCEPTION_HAPPENED: bool = false;

#[no_mangle]
static mut EXCEPTION_DATA: usize = 0;

#[no_mangle]
static mut UNWIND_INFO: Option<Box<HashMap<*const u8, UnwindEntry>>> = None;

pub struct CustomUnwinder(());

#[derive(Debug, Clone)]
struct UnwindEntry {
    landing_pad: *const u8,
    kind: UnwindEntryKind,
    //function_name: String,
}

#[derive(Debug, Copy, Clone)]
enum UnwindEntryKind {
    NoCleanup, // FIXME maybe represent as null landing pad?
    Cleanup,
    Catch,
}

impl CustomUnwinder {
    /// This unwinder is very fragile.
    pub unsafe fn new() -> CustomUnwinder {
        if UNWIND_INFO.is_none() {
            UNWIND_INFO = Some(Box::new(HashMap::new()));
        }
        CustomUnwinder(())
    }
}

unsafe impl Unwinder for CustomUnwinder {
    fn register_function(
        &mut self,
        module: &mut cranelift_jit::JITModule,
        func_id: cranelift_module::FuncId,
        context: &cranelift::codegen::Context,
    ) {
        module.finalize_definitions().unwrap();

        let func_addr = module.get_finalized_function(func_id);

        for call_site in context.compiled_code().unwrap().buffer.call_sites() {
            unsafe { UNWIND_INFO.as_mut() }.unwrap().insert(
                unsafe { func_addr.byte_add(call_site.ret_addr.try_into().unwrap()) },
                UnwindEntry {
                    landing_pad: if call_site.id.is_none() {
                        std::ptr::null()
                    } else {
                        unsafe {
                            func_addr.byte_add(call_site.alternate_targets[0].try_into().unwrap())
                        }
                    },
                    kind: match call_site.id.map(|id| id.bits()) {
                        None => UnwindEntryKind::NoCleanup,
                        Some(0) => UnwindEntryKind::Cleanup,
                        Some(1) => UnwindEntryKind::Catch,
                        _ => unreachable!(),
                    },
                    /*function_name: module
                    .declarations()
                    .get_function_decl(func_id)
                    .linkage_name(func_id)
                    .into_owned(),*/
                },
            );
        }
    }

    unsafe fn call_and_catch_unwind0(
        &self,
        func: extern "C-unwind" fn() -> usize,
    ) -> Result<usize, usize> {
        let res;

        EXCEPTION_HAPPENED = false;

        #[cfg(target_arch = "aarch64")]
        asm!("
            adrp x9, 1f // FIXME use an actual landing pad to return the exception data
            str x9, [x11]
            blr x10
            1:
            ",
            clobber_abi("C"),
            lateout("x0") res,

            // these all use scratch registers
            out("x9") _,
            in("x10") func,
            in("x11") &mut CURRENT_CALL_AND_UNWIND_RET_ADDR,
        );

        if EXCEPTION_HAPPENED {
            Err(res)
        } else {
            Ok(res)
        }
    }

    #[no_mangle]
    unsafe fn call_and_catch_unwind1(
        &self,
        func: extern "C-unwind" fn(usize) -> usize,
        arg: usize,
    ) -> Result<usize, usize> {
        //println!("{:#?}", UNWIND_INFO.as_ref().unwrap());

        let res;

        EXCEPTION_HAPPENED = false;

        #[cfg(target_arch = "aarch64")]
        asm!("
            adrp x9, 1f // FIXME use an actual landing pad to return the exception data
            str x9, [x11]
            blr x10
            1:
            ",
            clobber_abi("C"),
            in("x0") arg,
            lateout("x0") res,

            // these all use scratch registers
            out("x9") _,
            in("x10") func,
            in("x11") &mut CURRENT_CALL_AND_UNWIND_RET_ADDR,
        );

        if EXCEPTION_HAPPENED {
            Err(res)
        } else {
            Ok(res)
        }
    }

    unsafe fn call_and_catch_unwind2(
        &self,
        func: extern "C-unwind" fn(usize, usize) -> usize,
        arg0: usize,
        arg1: usize,
    ) -> Result<usize, usize> {
        let res;

        EXCEPTION_HAPPENED = false;

        #[cfg(target_arch = "aarch64")]
        asm!("
            adrp x9, 1f
            str x9, [x11]
            blr x10
            1:
            ",
            clobber_abi("C"),
            in("x0") arg0,
            in("x1") arg1,
            lateout("x0") res,

            // these all use scratch registers
            out("x9") _,
            in("x10") func,
            in("x11") &mut CURRENT_CALL_AND_UNWIND_RET_ADDR,
        );

        if EXCEPTION_HAPPENED {
            Err(res)
        } else {
            Ok(res)
        }
    }

    fn get_exception_data(&self, _builder: &mut FunctionBuilder, exception_val: Value) -> Value {
        exception_val
    }

    fn throw_func(&self) -> unsafe extern "C-unwind" fn(exception: usize) -> ! {
        do_throw
    }

    fn resume_unwind_func(
        &self,
    ) -> unsafe extern "C-unwind" fn(exception: *mut super::_Unwind_Exception) -> ! {
        do_resume_unwind
    }
}

#[naked]
unsafe extern "C-unwind" fn do_throw(exception: usize) -> ! {
    #[cfg(target_arch = "aarch64")]
    asm!(
        "
        // Store exception information
        .global EXCEPTION_DATA
        adrp x9, :got:EXCEPTION_DATA
        ldr x9, [x9, :got_lo12:EXCEPTION_DATA]
        str x0, [x9]

        .global EXCEPTION_HAPPENED
        adrp x9, :got:EXCEPTION_HAPPENED
        ldr x9, [x9, :got_lo12:EXCEPTION_HAPPENED]
        mov x10, #1
        str x10, [x9]

        // Find landing pad for caller
        // FIXME handle case where there is no landing pad
        .global unwind_custom_find_landing_pad
        mov x0, lr
        ldr x1, [sp], #16
        bl unwind_custom_find_landing_pad

        mov x10, x0

        .global EXCEPTION_DATA
        adrp x9, :got:EXCEPTION_DATA
        ldr x9, [x9, :got_lo12:EXCEPTION_DATA]
        ldr x0, [x9]

        mov lr, x10
        ret
        ",
        options(noreturn),
    );
}

#[naked]
unsafe extern "C-unwind" fn do_resume_unwind(exception: *mut super::_Unwind_Exception) -> ! {
    #[cfg(target_arch = "aarch64")]
    asm!(
        "
        1:
        // Unwind single frame
        ldp fp, lr, [sp], #16
        // FIXME restore registers

        // try to find the landing pad
        .global unwind_custom_find_landing_pad
        mov x0, lr
        bl unwind_custom_find_landing_pad

        cbz x0, 1b // no landing pad for current frame. continue unwinding.

        mov x10, x0

        .global EXCEPTION_DATA
        adrp x9, :got:EXCEPTION_DATA
        ldr x9, [x9, :got_lo12:EXCEPTION_DATA]
        ldr x0, [x9]

        mov lr, x10
        ret
        ",
        options(noreturn)
    );
}

#[no_mangle]
unsafe extern "C" fn unwind_custom_find_landing_pad(ip: *const u8) -> *const u8 {
    //println!("find landing pad for {ip:p}");
    if ip as usize == CURRENT_CALL_AND_UNWIND_RET_ADDR {
        //println!("at call_and_catch_unwind");
        return ip; // FIXME Maybe return landing pad?
    }
    let unwind_entry = UNWIND_INFO
        .as_ref()
        .unwrap()
        .get(&ip)
        .clone()
        .unwrap_or_else(|| std::process::abort());
    // println!(
    //     "landing pad of {} is at {:p}",
    //     unwind_entry.function_name, unwind_entry.landing_pad
    // );
    match unwind_entry.kind {
        UnwindEntryKind::NoCleanup => ip,
        UnwindEntryKind::Cleanup => unwind_entry.landing_pad,
        UnwindEntryKind::Catch => {
            EXCEPTION_HAPPENED = false;
            unwind_entry.landing_pad
        }
    }
}
