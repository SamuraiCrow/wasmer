//! Create, read, write, grow, destroy memory of an instance.

use crate::{error::update_last_error, error::CApiError, wasmer_limits_t, wasmer_result_t};
use std::cell::Cell;
use wasmer_runtime::Memory;
use wasmer_runtime_core::{
    types::MemoryDescriptor,
    units::{Bytes, Pages},
};

/// Opaque pointer to a `wasmer_runtime::Memory` value in Rust.
///
/// A `wasmer_runtime::Memory` represents a WebAssembly memory. It is
/// possible to create one with `wasmer_memory_new()` and pass it as
/// imports of an instance, or to read it from exports of an instance
/// with `wasmer_export_to_memory()`.
#[repr(C)]
#[derive(Clone)]
pub struct wasmer_memory_t;

/// Creates a new empty WebAssembly memory for the given descriptor.
///
/// The result is stored in the first argument `memory` if successful,
/// i.e. when the function returns
/// `wasmer_result_t::WASMER_OK`. Otherwise,
/// `wasmer_result_t::WASMER_ERROR` is returned, and
/// `wasmer_last_error_length()` with `wasmer_last_error_message()`
/// must be used to read the error message.
///
/// The caller owns the memory and is responsible to free it with
/// `wasmer_memory_destroy()`.
///
/// Example:
///
/// ```c
/// // 1. The memory object.
/// wasmer_memory_t *memory = NULL;
///
/// // 2. The memory descriptor.
/// wasmer_limits_t memory_descriptor = {
///     .min = 10,
///     .max = {
///         .has_some = true,
///         .some = 15,
///     },
/// };
///
/// // 3. Initialize the memory.
/// wasmer_result_t result = wasmer_memory_new(&memory, memory_descriptor);
///
/// if (result != WASMER_OK) {
///     int error_length = wasmer_last_error_length();
///     char *error = malloc(error_length);
///     wasmer_last_error_message(error, error_length);
///     // Do something with `error`…
/// }
///
/// // 4. Free the memory!
/// wasmer_memory_destroy(memory);
/// ```
#[no_mangle]
pub unsafe extern "C" fn wasmer_memory_new(
    memory: *mut *mut wasmer_memory_t,
    limits: wasmer_limits_t,
) -> wasmer_result_t {
    let max = if limits.max.has_some {
        Some(Pages(limits.max.some))
    } else {
        None
    };
    let desc = MemoryDescriptor::new(Pages(limits.min), max, false);
    let new_desc = match desc {
        Ok(desc) => desc,
        Err(error) => {
            update_last_error(CApiError {
                msg: error.to_string(),
            });
            return wasmer_result_t::WASMER_ERROR;
        }
    };
    let result = Memory::new(new_desc);
    let new_memory = match result {
        Ok(memory) => memory,
        Err(error) => {
            update_last_error(error);
            return wasmer_result_t::WASMER_ERROR;
        }
    };
    *memory = Box::into_raw(Box::new(new_memory)) as *mut wasmer_memory_t;
    wasmer_result_t::WASMER_OK
}

/// Grows a memory by the given number of pages (of 65Kb each).
///
/// The functions return `wasmer_result_t::WASMER_OK` upon success,
/// `wasmer_result_t::WASMER_ERROR` otherwise. Use
/// `wasmer_last_error_length()` with `wasmer_last_error_message()` to
/// read the error message.
///
/// Example:
///
/// ```c
/// wasmer_result_t result = wasmer_memory_grow(memory, 10 /* pages */);
///
/// if (result != WASMER_OK) {
///     // …
/// }
/// ```
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_memory_grow(memory: *mut wasmer_memory_t, delta: u32) -> wasmer_result_t {
    let memory = unsafe { &*(memory as *mut Memory) };
    let delta_result = memory.grow(Pages(delta));
    match delta_result {
        Ok(_) => wasmer_result_t::WASMER_OK,
        Err(grow_error) => {
            update_last_error(grow_error);
            wasmer_result_t::WASMER_ERROR
        }
    }
}

/// Reads the current length (in pages) of the given memory.
///
/// The function returns zero if `memory` is null.
///
/// Example:
///
/// ```c
/// uint32_t memory_length = wasmer_memory_length(memory);
///
/// printf("Memory pages length: %d\n", memory_length);
/// ```
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_memory_length(memory: *const wasmer_memory_t) -> u32 {
    if memory.is_null() {
        return 0;
    }

    let memory = unsafe { &*(memory as *const Memory) };
    let Pages(length) = memory.size();

    length
}

/// Gets the start pointer to the bytes within a Memory
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_memory_data(mem: *const wasmer_memory_t) -> *mut u8 {
    let memory = unsafe { &*(mem as *const Memory) };
    memory.view::<u8>()[..].as_ptr() as *mut Cell<u8> as *mut u8
}

/// Gets the size in bytes of a Memory
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_memory_data_length(mem: *mut wasmer_memory_t) -> u32 {
    let memory = mem as *mut Memory;
    let Bytes(len) = unsafe { (*memory).size().bytes() };
    len as u32
}

/// Frees memory for the given Memory
#[allow(clippy::cast_ptr_alignment)]
#[no_mangle]
pub extern "C" fn wasmer_memory_destroy(memory: *mut wasmer_memory_t) {
    if !memory.is_null() {
        unsafe { Box::from_raw(memory as *mut Memory) };
    }
}
