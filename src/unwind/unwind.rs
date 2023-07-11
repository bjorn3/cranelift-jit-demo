//! Unwind info generation (`.eh_frame`)

use cranelift::codegen::ir::{types, AbiParam, Signature};
use cranelift::codegen::isa::unwind::UnwindInfo;
use cranelift::codegen::Context;

use cranelift::prelude::{FunctionBuilder, InstBuilder, MemFlags, Value};
use cranelift_module::{DataId, FuncId, Linkage, Module};

use gimli::write::{EhFrame, FrameTable};
use gimli::RunTimeEndian;

use crate::unwind::unwind_fast::FastLandingpadStrategy;
use crate::unwind::unwind_gcc::GccLandingpadStrategy;
use crate::unwind::{Unwinder, _Unwind_Exception, _Unwind_RaiseException, _Unwind_Resume};

use super::emit::{address_for_data, address_for_func};

pub(crate) trait LandingpadStrategy {
    fn personality_name(&self) -> &str;
    fn personality_addr(&self) -> *const u8;
    fn generate_lsda(&self, module: &mut dyn Module, func_id: FuncId, context: &Context) -> DataId;
}

pub struct EhFrameUnwinder {
    strategy: Box<dyn LandingpadStrategy>,
}

impl EhFrameUnwinder {
    pub fn new_gcc() -> Self {
        EhFrameUnwinder {
            strategy: Box::new(GccLandingpadStrategy),
        }
    }

    pub fn new_fast() -> Self {
        EhFrameUnwinder {
            strategy: Box::new(FastLandingpadStrategy),
        }
    }
}

unsafe impl Unwinder for EhFrameUnwinder {
    fn register_function(
        &mut self,
        module: &mut cranelift_jit::JITModule,
        func_id: FuncId,
        context: &Context,
    ) {
        let mut frame_table = FrameTable::default();

        let cie_id = if let Some(mut cie) = module.isa().create_systemv_cie() {
            cie.fde_address_encoding = gimli::DW_EH_PE_absptr;
            cie.lsda_encoding = Some(gimli::DW_EH_PE_absptr);

            // FIXME use eh_personality lang item instead
            let personality = module
                .declare_function(
                    self.strategy.personality_name(),
                    Linkage::Import,
                    &Signature {
                        params: vec![
                            AbiParam::new(types::I32),
                            AbiParam::new(types::I32),
                            AbiParam::new(types::I64),
                            AbiParam::new(module.target_config().pointer_type()),
                            AbiParam::new(module.target_config().pointer_type()),
                        ],
                        returns: vec![AbiParam::new(types::I32)],
                        call_conv: module.target_config().default_call_conv,
                    },
                )
                .unwrap();

            cie.personality = Some((gimli::DW_EH_PE_absptr, address_for_func(personality)));
            Some(frame_table.add_cie(cie))
        } else {
            None
        };

        let unwind_info = if let Some(unwind_info) = context
            .compiled_code()
            .unwrap()
            .create_unwind_info(module.isa())
            .unwrap()
        {
            unwind_info
        } else {
            return;
        };

        match unwind_info {
            UnwindInfo::SystemV(unwind_info) => {
                let mut fde = unwind_info.to_fde(address_for_func(func_id));

                let lsda = self.strategy.generate_lsda(module, func_id, context);

                fde.lsda = Some(address_for_data(lsda));
                frame_table.add_fde(cie_id.unwrap(), fde);
            }
            UnwindInfo::WindowsX64(_) => {
                // FIXME implement this
            }
            unwind_info => unimplemented!("{:?}", unwind_info),
        }

        module.finalize_definitions().unwrap();

        use std::mem::ManuallyDrop;

        let mut eh_frame = EhFrame::from(super::emit::WriterRelocate::new(
            if cfg!(target_endian = "little") {
                RunTimeEndian::Little
            } else {
                RunTimeEndian::Big
            },
        ));
        frame_table.write_eh_frame(&mut eh_frame).unwrap();

        if eh_frame.0.writer.slice().is_empty() {
            return;
        }

        let mut eh_frame = eh_frame.0.relocate_for_jit(module, &*self.strategy);

        // GCC expects a terminating "empty" length, so write a 0 length at the end of the table.
        eh_frame.extend(&[0, 0, 0, 0]);

        // FIXME support unregistering unwind tables once cranelift-jit supports deallocating
        // individual functions
        let eh_frame = ManuallyDrop::new(eh_frame);

        unsafe {
            // =======================================================================
            // Everything after this line up to the end of the file is loosely based on
            // https://github.com/bytecodealliance/wasmtime/blob/4471a82b0c540ff48960eca6757ccce5b1b5c3e4/crates/jit/src/unwind/systemv.rs
            #[cfg(target_os = "macos")]
            {
                // On macOS, `__register_frame` takes a pointer to a single FDE
                let start = eh_frame.as_ptr();
                let end = start.add(eh_frame.len());
                let mut current = start;

                // Walk all of the entries in the frame table and register them
                while current < end {
                    let len = std::ptr::read::<u32>(current as *const u32) as usize;

                    // Skip over the CIE
                    if current != start {
                        __register_frame(current);
                    }

                    // Move to the next table entry (+4 because the length itself is not inclusive)
                    current = current.add(len + 4);
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                // On other platforms, `__register_frame` will walk the FDEs until an entry of length 0
                __register_frame(eh_frame.as_ptr());
            }
        }
    }

    unsafe fn call_and_catch_unwind0(
        &self,
        func: extern "C-unwind" fn() -> usize,
    ) -> Result<usize, usize> {
        std::panic::catch_unwind(|| func()).map_err(|_err| {
            todo!("get exception data");
        })
    }

    unsafe fn call_and_catch_unwind1(
        &self,
        func: extern "C-unwind" fn(usize) -> usize,
        arg: usize,
    ) -> Result<usize, usize> {
        std::panic::catch_unwind(|| func(arg)).map_err(|_err| {
            todo!("get exception data");
        })
    }

    unsafe fn call_and_catch_unwind2(
        &self,
        func: extern "C-unwind" fn(usize, usize) -> usize,
        arg0: usize,
        arg1: usize,
    ) -> Result<usize, usize> {
        std::panic::catch_unwind(|| func(arg0, arg1)).map_err(|_err| {
            todo!("get exception data");
        })
    }

    fn get_exception_data(&self, builder: &mut FunctionBuilder, exception_val: Value) -> Value {
        builder
            .ins()
            .load(types::I64, MemFlags::trusted(), exception_val, 32)
    }

    fn throw_func(&self) -> unsafe extern "C-unwind" fn(exception: usize) -> ! {
        do_throw
    }

    fn resume_unwind_func(
        &self,
    ) -> unsafe extern "C-unwind" fn(exception: *mut _Unwind_Exception) -> ! {
        do_resume_unwind
    }
}

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

extern "C" {
    // libunwind import
    fn __register_frame(fde: *const u8);
}
