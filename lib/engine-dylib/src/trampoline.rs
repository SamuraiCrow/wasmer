//! Trampolines for libcalls.
//!
//! This is needed because the target of libcall relocations are not reachable
//! through normal branch instructions.
//!
//! There is an additional complexity for dynamic libraries: we can't just
//! import the symbol from the host executable because executables don't export
//! dynamic symbols (it's possible but requires special linker options).
//!
//! Instead, we export a table of function pointers in the data section which is
//! manually filled in by the runtime after the dylib is loaded.

use enum_iterator::IntoEnumIterator;
use object::{
    elf, macho,
    write::{Object, Relocation, SectionId, StandardSection, Symbol, SymbolId, SymbolSection},
    RelocationEncoding, RelocationKind, SymbolFlags, SymbolKind, SymbolScope,
};
use wasmer_compiler::{Architecture, BinaryFormat, Target};
use wasmer_vm::libcalls::LibCall;

/// Symbol exported from the dynamic library which points to the trampoline table.
pub const WASMER_TRAMPOLINES_SYMBOL: &[u8] = b"WASMER_TRAMPOLINES";

/// Internal symbol with local scope that can't be preempted by external symbols.
const WASMER_TRAMPOLINES_SYMBOL_INTERNAL: &[u8] = b"WASMER_TRAMPOLINES_INTERNAL";

// SystemV says that both x16 and x17 are available as intra-procedural scratch
// registers but Apple's ABI restricts us to use x17.
// ADRP x17, #...        11 00 00 90
// LDR x17, [x17, #...]  31 02 40 f9
// BR x17                20 02 1f d6
const AARCH64_TRAMPOLINE: [u8; 12] = [
    0x11, 0x00, 0x00, 0x90, 0x31, 0x02, 0x40, 0xf9, 0x20, 0x02, 0x1f, 0xd6,
];

// JMP [RIP + ...]   FF 25 00 00 00 00
const X86_64_TRAMPOLINE: [u8; 6] = [0xff, 0x25, 0x00, 0x00, 0x00, 0x00];

fn emit_trampoline(
    obj: &mut Object,
    text: SectionId,
    trampoline_table: SymbolId,
    libcall: LibCall,
    target: &Target,
) {
    let function_name = libcall.to_function_name();
    let libcall_symbol = obj.add_symbol(Symbol {
        name: function_name.as_bytes().to_vec(),
        value: 0,
        size: 0,
        kind: SymbolKind::Text,
        scope: SymbolScope::Compilation,
        weak: false,
        section: SymbolSection::Section(text),
        flags: SymbolFlags::None,
    });

    match target.triple().architecture {
        Architecture::Aarch64(_) => {
            let (reloc1, reloc2) = match target.triple().binary_format {
                BinaryFormat::Elf => (
                    RelocationKind::Elf(elf::R_AARCH64_ADR_PREL_PG_HI21),
                    RelocationKind::Elf(elf::R_AARCH64_LDST64_ABS_LO12_NC),
                ),
                BinaryFormat::Macho => (
                    RelocationKind::MachO {
                        value: macho::ARM64_RELOC_PAGE21,
                        relative: true,
                    },
                    RelocationKind::MachO {
                        value: macho::ARM64_RELOC_PAGEOFF12,
                        relative: false,
                    },
                ),
                _ => panic!("Unsupported binary format on AArch64"),
            };
            let offset = obj.add_symbol_data(libcall_symbol, text, &AARCH64_TRAMPOLINE, 4);
            obj.add_relocation(
                text,
                Relocation {
                    offset,
                    size: 32,
                    kind: reloc1,
                    encoding: RelocationEncoding::Generic,
                    symbol: trampoline_table,
                    addend: libcall as i64 * 8,
                },
            )
            .unwrap();
            obj.add_relocation(
                text,
                Relocation {
                    offset: offset + 4,
                    size: 32,
                    kind: reloc2,
                    encoding: RelocationEncoding::Generic,
                    symbol: trampoline_table,
                    addend: libcall as i64 * 8,
                },
            )
            .unwrap();
        }
        Architecture::X86_64 => {
            let offset = obj.add_symbol_data(libcall_symbol, text, &X86_64_TRAMPOLINE, 1);
            obj.add_relocation(
                text,
                Relocation {
                    offset: offset + 2,
                    size: 32,
                    kind: RelocationKind::Relative,
                    encoding: RelocationEncoding::Generic,
                    symbol: trampoline_table,
                    // -4 because RIP-relative addressing starts from the end of
                    // the instruction.
                    addend: libcall as i64 * 8 - 4,
                },
            )
            .unwrap();
        }
        arch => panic!("Unsupported architecture: {}", arch),
    };
}

/// Emits the libcall trampolines and table to the object file.
pub fn emit_trampolines(obj: &mut Object, target: &Target) {
    let text = obj.section_id(StandardSection::Text);
    let bss = obj.section_id(StandardSection::UninitializedData);

    // We need to create 2 symbols here:
    // - one with local scope that the trampolines point to.
    // - one with global scope that is exported from the dynamic library.
    //
    // This is necessary because we can't point to a global scope symbol with
    // non-GOT relocations.
    let trampoline_table = obj.add_symbol(Symbol {
        name: WASMER_TRAMPOLINES_SYMBOL.to_vec(),
        value: 0,
        size: 0,
        kind: SymbolKind::Data,
        scope: SymbolScope::Dynamic,
        weak: false,
        section: SymbolSection::Section(bss),
        flags: SymbolFlags::None,
    });
    obj.add_symbol_bss(trampoline_table, bss, LibCall::VARIANT_COUNT as u64 * 8, 8);

    // Make an internal alias of WASMER_TRAMPOLINES_SYMBOL.
    let trampoline_table_internal = obj.add_symbol(Symbol {
        name: WASMER_TRAMPOLINES_SYMBOL_INTERNAL.to_vec(),
        value: 0,
        size: 0,
        kind: SymbolKind::Data,
        scope: SymbolScope::Compilation,
        weak: false,
        section: SymbolSection::Section(bss),
        flags: SymbolFlags::None,
    });
    let &Symbol { value, size, .. } = obj.symbol(trampoline_table);
    obj.set_symbol_data(trampoline_table_internal, text, value, size);

    for libcall in LibCall::into_enum_iter() {
        emit_trampoline(obj, text, trampoline_table_internal, libcall, target);
    }
}

/// Fills in the libcall trampoline table at the given address.
pub unsafe fn fill_trampoline_table(table: *mut usize) {
    for libcall in LibCall::into_enum_iter() {
        *table.add(libcall as usize) = libcall.function_pointer();
    }
}
