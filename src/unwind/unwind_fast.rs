use std::{mem, ptr};

use cranelift::codegen::Context;
use cranelift_module::{DataDescription, DataId, FuncId, Module};

use crate::unwind::unwind::LandingpadStrategy;
use crate::unwind::{
    _Unwind_Action, _Unwind_Context, _Unwind_Exception, _Unwind_Exception_Class, _Unwind_GetIP,
    _Unwind_GetLanguageSpecificData, _Unwind_Reason_Code, _Unwind_SetGR, _Unwind_SetIP,
    UNWIND_DATA_REG,
};

const ENTRY_KIND_NO_CLEANUP: u8 = 1;
const ENTRY_KIND_CLEANUP: u8 = 2;
const ENTRY_KIND_CATCH: u8 = 3;

pub(crate) struct FastLandingpadStrategy;

impl LandingpadStrategy for FastLandingpadStrategy {
    fn personality_name(&self) -> &str {
        "__jit_eh_personality"
    }

    fn personality_addr(&self) -> *const u8 {
        jit_eh_personality as *const u8
    }

    fn generate_lsda(&self, module: &mut dyn Module, func_id: FuncId, context: &Context) -> DataId {
        let lsda = module.declare_anonymous_data(false, false).unwrap();

        let mut lsda_data = vec![];

        lsda_data.extend(usize::to_ne_bytes(0)); // placeholder for function address

        for call_site in context.compiled_code().unwrap().buffer.call_sites() {
            lsda_data.extend(u32::to_ne_bytes(call_site.ret_addr));

            match call_site.id.map(|id| id.bits()) {
                None => {
                    lsda_data.push(ENTRY_KIND_NO_CLEANUP);
                    lsda_data.extend([0; 4]);
                }
                Some(0) => {
                    lsda_data.push(ENTRY_KIND_CLEANUP);
                    lsda_data.extend(u32::to_ne_bytes(call_site.alternate_targets[0]));
                }
                Some(1) => {
                    lsda_data.push(ENTRY_KIND_CATCH);
                    lsda_data.extend(u32::to_ne_bytes(call_site.alternate_targets[0]));
                }
                _ => unreachable!(),
            }
        }

        lsda_data.extend([0; 4]); // end marker

        let mut data = DataDescription::new();
        data.define(lsda_data.into_boxed_slice());
        let func_ref = module.declare_func_in_data(func_id, &mut data);
        data.write_function_addr(0, func_ref);

        module.define_data(lsda, &data).unwrap();

        lsda
    }
}

unsafe extern "C" fn jit_eh_personality(
    _version: i32,
    actions: _Unwind_Action,
    _exception_class: _Unwind_Exception_Class,
    exception_object: *mut _Unwind_Exception,
    context: *mut _Unwind_Context,
) -> _Unwind_Reason_Code {
    let ip = _Unwind_GetIP(context);
    let lsda = _Unwind_GetLanguageSpecificData(context);

    let func_start = ptr::read_unaligned(lsda as *const usize);
    let func_offset = u32::try_from(ip - func_start).unwrap();

    let mut entry = lsda.byte_add(mem::size_of::<usize>());
    loop {
        let entry_func_offset = ptr::read_unaligned(entry as *const u32);
        if entry_func_offset == 0 {
            panic!("Call site not found");
        }
        if entry_func_offset != func_offset {
            entry = entry.byte_add(4 + 1 + 4);
            continue;
        }
        let entry_kind = ptr::read_unaligned(entry.byte_add(4) as *const u8);
        let entry_landing_pad = ptr::read_unaligned(entry.byte_add(4 + 1) as *const u32);
        if actions as i32 & _Unwind_Action::_UA_SEARCH_PHASE as i32 != 0 {
            match entry_kind {
                ENTRY_KIND_NO_CLEANUP => return _Unwind_Reason_Code::_URC_CONTINUE_UNWIND,
                ENTRY_KIND_CLEANUP => return _Unwind_Reason_Code::_URC_CONTINUE_UNWIND,
                ENTRY_KIND_CATCH => return _Unwind_Reason_Code::_URC_HANDLER_FOUND,
                _ => unreachable!(),
            }
        } else {
            match entry_kind {
                ENTRY_KIND_NO_CLEANUP => return _Unwind_Reason_Code::_URC_CONTINUE_UNWIND,
                ENTRY_KIND_CLEANUP | ENTRY_KIND_CATCH => {
                    _Unwind_SetGR(context, UNWIND_DATA_REG.0, exception_object as usize);
                    _Unwind_SetGR(context, UNWIND_DATA_REG.1, 0);
                    _Unwind_SetIP(context, func_start + entry_landing_pad as usize);
                    return _Unwind_Reason_Code::_URC_INSTALL_CONTEXT;
                }
                _ => unreachable!(),
            }
        }
    }
}
