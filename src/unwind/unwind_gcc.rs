use cranelift::codegen::Context;
use cranelift_module::{DataDescription, DataId, FuncId, Module};
use eh_frame_experiments::{
    Action, ActionTable, CallSite, CallSiteTable, ExceptionSpecTable, GccExceptTable, TypeInfoTable,
};
use gimli::write::Address;
use gimli::{Encoding, Format, RunTimeEndian};

use crate::unwind::unwind::LandingpadStrategy;
use crate::unwind::{
    DebugRelocName, _Unwind_Action, _Unwind_Context, _Unwind_Exception, _Unwind_Exception_Class,
    _Unwind_GetIP, _Unwind_GetLanguageSpecificData, _Unwind_Reason_Code,
};

pub(crate) struct GccLandingpadStrategy;

impl LandingpadStrategy for GccLandingpadStrategy {
    fn personality_name(&self) -> &str {
        "__jit_eh_personality"
    }

    fn personality_addr(&self) -> *const u8 {
        jit_eh_personality as *const u8
    }

    fn generate_lsda(&self, module: &mut dyn Module, context: &Context) -> DataId {
        // FIXME use unique symbol name derived from function name
        let lsda = module.declare_anonymous_data(false, false).unwrap();

        let encoding = Encoding {
            format: Format::Dwarf32,
            version: 1,
            address_size: module.isa().frontend_config().pointer_bytes(),
        };

        // FIXME add actual exception information here
        let mut gcc_except_table_data = GccExceptTable {
            call_sites: CallSiteTable(vec![]),
            actions: ActionTable::new(),
            type_info: TypeInfoTable::new(gimli::DW_EH_PE_udata4),
            exception_specs: ExceptionSpecTable::new(),
        };

        let catch_type = gcc_except_table_data.type_info.add(Address::Constant(0));
        let catch_action = gcc_except_table_data.actions.add(Action {
            kind: eh_frame_experiments::ActionKind::Catch(catch_type),
            next_action: None,
        });

        //println!("{:?}", context.compiled_code().unwrap().buffer.call_sites());
        for call_site in context.compiled_code().unwrap().buffer.call_sites() {
            match call_site.id.map(|id| id.bits()) {
                None => gcc_except_table_data.call_sites.0.push(CallSite {
                    start: u64::from(call_site.ret_addr - 1),
                    length: 1,
                    landing_pad: 0,
                    action_entry: None,
                }),
                Some(0) => gcc_except_table_data.call_sites.0.push(CallSite {
                    start: u64::from(call_site.ret_addr - 1),
                    length: 1,
                    landing_pad: u64::from(call_site.alternate_targets[0]),
                    action_entry: None,
                }),
                Some(1) => gcc_except_table_data.call_sites.0.push(CallSite {
                    start: u64::from(call_site.ret_addr - 1),
                    length: 1,
                    landing_pad: u64::from(call_site.alternate_targets[0]),
                    action_entry: Some(catch_action),
                }),
                _ => unreachable!(),
            }
        }
        //println!("{gcc_except_table_data:?}");

        let mut gcc_except_table =
            super::emit::WriterRelocate::new(if cfg!(target_endian = "little") {
                RunTimeEndian::Little
            } else {
                RunTimeEndian::Big
            });

        gcc_except_table_data
            .write(&mut gcc_except_table, encoding)
            .unwrap();

        let mut data = DataDescription::new();
        data.define(gcc_except_table.writer.into_vec().into_boxed_slice());
        data.set_segment_section("", ".gcc_except_table");

        for reloc in &gcc_except_table.relocs {
            match reloc.name {
                DebugRelocName::Section(_id) => unreachable!(),
                DebugRelocName::Symbol(id) => {
                    let id = id.try_into().unwrap();
                    if id & 1 << 31 == 0 {
                        let func_ref = module.declare_func_in_data(FuncId::from_u32(id), &mut data);
                        data.write_function_addr(reloc.offset, func_ref);
                    } else {
                        let gv = module
                            .declare_data_in_data(DataId::from_u32(id & !(1 << 31)), &mut data);
                        data.write_data_addr(reloc.offset, gv, 0);
                    }
                }
            };
        }

        module.define_data(lsda, &data).unwrap();

        lsda
    }
}

extern "C" {
    fn rust_eh_personality(
        version: i32,
        actions: _Unwind_Action,
        exception_class: _Unwind_Exception_Class,
        exception_object: *mut _Unwind_Exception,
        context: *mut _Unwind_Context,
    ) -> _Unwind_Reason_Code;
}

unsafe extern "C" fn jit_eh_personality(
    version: i32,
    actions: _Unwind_Action,
    exception_class: _Unwind_Exception_Class,
    exception_object: *mut _Unwind_Exception,
    context: *mut _Unwind_Context,
) -> _Unwind_Reason_Code {
    unsafe {
        // XXX This depends on unstable implementation details of rustc
        return rust_eh_personality(version, actions, exception_class, exception_object, context);
    }

    // FIXME Maybe implement our own personality function?
    let ip = _Unwind_GetIP(context);
    let lsda = _Unwind_GetLanguageSpecificData(context);

    if actions as i32 & _Unwind_Action::_UA_SEARCH_PHASE as i32 != 0 {
        println!("personality for {:#x}; lsda={:p}; search", ip, lsda,);

        panic!();
    } else if actions as i32 & _Unwind_Action::_UA_CLEANUP_PHASE as i32 != 0 {
        println!("personality for {:#x}; lsda={:p}; cleanup", ip, lsda,);

        panic!();
    } else {
        panic!();
    }
}
