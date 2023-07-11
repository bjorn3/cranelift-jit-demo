#![feature(c_unwind)]

use core::mem;
use cranelift_jit_demo::jit;
use std::time::Instant;

fn main() {
    match (|| {
        println!("With GCC personality:");
        run_tests(jit::JIT::new(Box::new(
            cranelift_jit_demo::unwind::EhFrameUnwinder::new_fast(),
        )))?;
        println!();

        println!("With fast personality:");
        run_tests(jit::JIT::new(Box::new(
            cranelift_jit_demo::unwind::EhFrameUnwinder::new_fast(),
        )))?;
        println!();

        println!("With custom unwinder:");
        run_tests(jit::JIT::new(Box::new(unsafe {
            cranelift_jit_demo::unwind::CustomUnwinder::new()
        })))?;
        println!();

        Ok::<(), String>(())
    })() {
        Ok(()) => {}
        Err(err) => println!("Error: {err}"),
    }
}

fn run_tests(mut jit: jit::JIT) -> Result<(), String> {
    println!("the answer is: {}", run_foo(&mut jit)?);
    println!(
        "recursive_fib(10) = {}",
        run_recursive_fib_code(&mut jit, 10)?
    );
    println!(
        "iterative_fib(10) = {}",
        run_iterative_fib_code(&mut jit, 10)?
    );
    println!("try_catch(1) = {}", run_try_catch(&mut jit, 1)?);
    run_hello(&mut jit)?;

    bench_call(&mut jit)?;
    bench_throw_single_unwind(&mut jit)?;

    Ok::<(), String>(())
}

fn run_foo(jit: &mut jit::JIT) -> Result<usize, String> {
    unsafe { run_code2(jit, FOO_CODE, 1, 0) }
}

fn run_recursive_fib_code(jit: &mut jit::JIT, input: usize) -> Result<usize, String> {
    unsafe { run_code1(jit, RECURSIVE_FIB_CODE, input) }
}

fn run_iterative_fib_code(jit: &mut jit::JIT, input: usize) -> Result<usize, String> {
    unsafe { run_code1(jit, ITERATIVE_FIB_CODE, input) }
}

fn run_try_catch(jit: &mut jit::JIT, input: usize) -> Result<usize, String> {
    jit.compile(DO_THROW_CODE)?;
    unsafe { run_code1(jit, TRY_CATCH_CODE, input) }
}

fn run_hello(jit: &mut jit::JIT) -> Result<usize, String> {
    jit.create_data("hello_string", "hello world!\0".as_bytes().to_vec())?;
    unsafe { run_code0(jit, HELLO_CODE) }
}

fn bench_call(jit: &mut jit::JIT) -> Result<(), String> {
    unsafe {
        jit.compile(NOP_FUNC_CODE)?;

        let code_ptr = jit.compile(BENCH_CALL_CODE)?;
        let code_fn = mem::transmute::<_, extern "C-unwind" fn() -> usize>(code_ptr);

        let start = Instant::now();
        jit.unwinder.call_and_catch_unwind0(code_fn).unwrap();
        println!("100_000_000 calls took {:?}", start.elapsed());

        Ok(())
    }
}

fn bench_throw_single_unwind(jit: &mut jit::JIT) -> Result<(), String> {
    unsafe {
        let code_ptr = jit.compile(BENCH_THROW_SINGLE_UNWIND_CODE)?;
        let code_fn = mem::transmute::<_, extern "C-unwind" fn() -> usize>(code_ptr);

        let start = Instant::now();
        jit.unwinder.call_and_catch_unwind0(code_fn).unwrap();
        println!("100_000 throws unwinding a single frame took {:?}", start.elapsed());

        Ok(())
    }
}

unsafe fn run_code0(jit: &mut jit::JIT, code: &str) -> Result<usize, String> {
    let code_ptr = jit.compile(code)?;
    let code_fn = mem::transmute::<_, extern "C-unwind" fn() -> usize>(code_ptr);
    Ok(jit.unwinder.call_and_catch_unwind0(code_fn).unwrap())
}

unsafe fn run_code1(jit: &mut jit::JIT, code: &str, input: usize) -> Result<usize, String> {
    let code_ptr = jit.compile(code)?;
    let code_fn = mem::transmute::<_, extern "C-unwind" fn(usize) -> usize>(code_ptr);
    Ok(jit.unwinder.call_and_catch_unwind1(code_fn, input).unwrap())
}

unsafe fn run_code2(
    jit: &mut jit::JIT,
    code: &str,
    input0: usize,
    input1: usize,
) -> Result<usize, String> {
    let code_ptr = jit.compile(code)?;
    let code_fn = mem::transmute::<_, extern "C-unwind" fn(usize, usize) -> usize>(code_ptr);
    Ok(jit
        .unwinder
        .call_and_catch_unwind2(code_fn, input0, input1)
        .unwrap())
}

// A small test function.
//
// The `(c)` declares a return variable; the function returns whatever value
// it was assigned when the function exits. Note that there are multiple
// assignments, so the input is not in SSA form, but that's ok because
// Cranelift handles all the details of translating into SSA form itself.
const FOO_CODE: &str = r#"
    fn foo(a, b) -> (c) {
        c = if a {
            if b {
                30
            } else {
                40
            }
        } else {
            50
        }
        c = c + 2
    }
"#;

/// Another example: Recursive fibonacci.
const RECURSIVE_FIB_CODE: &str = r#"
    fn recursive_fib(n) -> (r) {
        r = if n == 0 {
                    0
            } else {
                if n == 1 {
                    1
                } else {
                    recursive_fib(n - 1) + recursive_fib(n - 2)
                }
            }
    }
"#;

/// Another example: Iterative fibonacci.
const ITERATIVE_FIB_CODE: &str = r#"
    fn iterative_fib(n) -> (r) {
        if n == 0 {
            r = 0
        } else {
            n = n - 1
            a = 0
            r = 1
            while n != 0 {
                t = r
                r = r + a
                a = t
                n = n - 1
            }
        }
    }
"#;

const DO_THROW_CODE: &str = r#"
    fn do_throw() -> (r) {
        throw 42
    }
"#;

const TRY_CATCH_CODE: &str = r#"
    fn try_catch(n) -> (r) {
        c = 0
        try {
            try {
                do_throw()
            } finally {
                c = 1
            }
        } catch e {
            r = e + c
        }
    }
"#;

/// Let's say hello, by calling into libc. The puts function is resolved by
/// dlsym to the libc function, and the string &hello_string is defined below.
const HELLO_CODE: &str = r#"
fn hello() -> (r) {
    puts(&hello_string)
}
"#;

const NOP_FUNC_CODE: &str = r#"
    fn nop() -> (r) {
        r = 0
    }
"#;

const BENCH_CALL_CODE: &str = r#"
    fn bench_call() -> (r) {
        n = 100000000
        while n != 0 {
            nop()
            n = n - 1
        }
    }
"#;

const BENCH_THROW_SINGLE_UNWIND_CODE: &str = r#"
    fn bench_throw_single_unwind() -> (r) {
        n = 100000
        while n != 0 {
            try {
                do_throw()
            } catch e {
                a = 0
            }
            n = n - 1
        }
    }
"#;
