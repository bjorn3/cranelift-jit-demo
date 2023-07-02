//! Write the debuginfo into an object file.

use cranelift_module::{DataId, FuncId, FuncOrDataId, Module};

use gimli::write::{Address, EndianVec, Result, Writer};
use gimli::{RunTimeEndian, SectionId};

use crate::unwind::unwind::LandingpadStrategy;

pub(super) fn address_for_func(func_id: FuncId) -> Address {
    let symbol = func_id.as_u32();
    assert!(symbol & 1 << 31 == 0);
    Address::Symbol {
        symbol: symbol as usize,
        addend: 0,
    }
}

pub(super) fn address_for_data(data_id: DataId) -> Address {
    let symbol = data_id.as_u32();
    assert!(symbol & 1 << 31 == 0);
    Address::Symbol {
        symbol: (symbol | 1 << 31) as usize,
        addend: 0,
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DebugReloc {
    pub(crate) offset: u32,
    pub(crate) size: u8,
    pub(crate) name: DebugRelocName,
    pub(crate) addend: i64,
    pub(crate) kind: object::RelocationKind,
}

#[derive(Debug, Clone)]
pub(crate) enum DebugRelocName {
    Section(SectionId),
    Symbol(usize),
}

/// A [`Writer`] that collects all necessary relocations.
#[derive(Clone)]
pub(super) struct WriterRelocate {
    pub(super) relocs: Vec<DebugReloc>,
    pub(super) writer: EndianVec<RunTimeEndian>,
}

impl WriterRelocate {
    pub(super) fn new(endian: RunTimeEndian) -> Self {
        WriterRelocate {
            relocs: Vec::new(),
            writer: EndianVec::new(endian),
        }
    }

    /// Perform the collected relocations to be usable for JIT usage.
    pub(super) fn relocate_for_jit(
        mut self,
        jit_module: &cranelift_jit::JITModule,
        strategy: &dyn LandingpadStrategy,
    ) -> Vec<u8> {
        let eh_personality_sym = match jit_module
            .declarations()
            .get_name(strategy.personality_name())
        {
            Some(FuncOrDataId::Func(func_id)) => Some(func_id.as_u32() as usize),
            Some(FuncOrDataId::Data(_)) => unreachable!(),
            None => None,
        };

        for reloc in self.relocs.drain(..) {
            assert!(reloc.kind == object::RelocationKind::Absolute);
            match reloc.name {
                DebugRelocName::Section(_) => unreachable!(),
                DebugRelocName::Symbol(sym) => {
                    let addr = if Some(sym) == eh_personality_sym {
                        strategy.personality_addr()
                    } else if sym & 1 << 31 == 0 {
                        jit_module.get_finalized_function(cranelift_module::FuncId::from_u32(
                            sym.try_into().unwrap(),
                        ))
                    } else {
                        jit_module
                            .get_finalized_data(cranelift_module::DataId::from_u32(
                                u32::try_from(sym).unwrap() & !(1 << 31),
                            ))
                            .0
                    };
                    let val = (addr as u64 as i64 + reloc.addend) as u64;
                    self.writer
                        .write_udata_at(reloc.offset as usize, val, reloc.size)
                        .unwrap();
                }
            }
        }
        self.writer.into_vec()
    }
}

impl Writer for WriterRelocate {
    type Endian = RunTimeEndian;

    fn endian(&self) -> Self::Endian {
        self.writer.endian()
    }

    fn len(&self) -> usize {
        self.writer.len()
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write(bytes)
    }

    fn write_at(&mut self, offset: usize, bytes: &[u8]) -> Result<()> {
        self.writer.write_at(offset, bytes)
    }

    fn write_address(&mut self, address: Address, size: u8) -> Result<()> {
        match address {
            Address::Constant(val) => self.write_udata(val, size),
            Address::Symbol { symbol, addend } => {
                let offset = self.len() as u64;
                self.relocs.push(DebugReloc {
                    offset: offset as u32,
                    size,
                    name: DebugRelocName::Symbol(symbol),
                    addend,
                    kind: object::RelocationKind::Absolute,
                });
                self.write_udata(0, size)
            }
        }
    }

    fn write_offset(&mut self, val: usize, section: SectionId, size: u8) -> Result<()> {
        let offset = self.len() as u32;
        self.relocs.push(DebugReloc {
            offset,
            size,
            name: DebugRelocName::Section(section),
            addend: val as i64,
            kind: object::RelocationKind::Absolute,
        });
        self.write_udata(0, size)
    }

    fn write_offset_at(
        &mut self,
        offset: usize,
        val: usize,
        section: SectionId,
        size: u8,
    ) -> Result<()> {
        self.relocs.push(DebugReloc {
            offset: offset as u32,
            size,
            name: DebugRelocName::Section(section),
            addend: val as i64,
            kind: object::RelocationKind::Absolute,
        });
        self.write_udata_at(offset, 0, size)
    }

    fn write_eh_pointer(&mut self, address: Address, eh_pe: gimli::DwEhPe, size: u8) -> Result<()> {
        match address {
            // Address::Constant arm copied from gimli
            Address::Constant(val) => {
                // Indirect doesn't matter here.
                let val = match eh_pe.application() {
                    gimli::DW_EH_PE_absptr => val,
                    gimli::DW_EH_PE_pcrel => {
                        // FIXME better handling of sign
                        let offset = self.len() as u64;
                        offset.wrapping_sub(val)
                    }
                    _ => {
                        return Err(gimli::write::Error::UnsupportedPointerEncoding(eh_pe));
                    }
                };
                self.write_eh_pointer_data(val, eh_pe.format(), size)
            }
            Address::Symbol { symbol, addend } => match eh_pe.application() {
                gimli::DW_EH_PE_absptr => {
                    self.relocs.push(DebugReloc {
                        offset: self.len() as u32,
                        size: 8, // pointer size
                        name: DebugRelocName::Symbol(symbol),
                        addend,
                        kind: object::RelocationKind::Absolute,
                    });
                    self.write_udata(0, size)
                }
                gimli::DW_EH_PE_pcrel => {
                    let size = match eh_pe.format() {
                        gimli::DW_EH_PE_sdata4 => 4,
                        gimli::DW_EH_PE_sdata8 => 8,
                        _ => return Err(gimli::write::Error::UnsupportedPointerEncoding(eh_pe)),
                    };
                    self.relocs.push(DebugReloc {
                        offset: self.len() as u32,
                        size,
                        name: DebugRelocName::Symbol(symbol),
                        addend,
                        kind: object::RelocationKind::Relative,
                    });
                    self.write_udata(0, size)
                }
                _ => Err(gimli::write::Error::UnsupportedPointerEncoding(eh_pe)),
            },
        }
    }
}
