/*
 * The Shadow Simulator
 * See LICENSE for licensing information
 */

struct ListenArguments {
    fd: libc::c_int,
    backlog: libc::c_int,
}

#[derive(Debug, Copy, Clone)]
struct BindAddress {
    address: libc::in_addr_t,
    port: libc::in_port_t,
}

/// A boxed function to run as a test.
type TestFn = Box<dyn Fn() -> Result<(), String>>;

fn main() {
    // should we run only tests that shadow supports
    let run_only_passing_tests = std::env::args().any(|x| x == "--shadow-passing");
    // should we summarize the results rather than exit on a failed test
    let summarize = std::env::args().any(|x| x == "--summarize");

    let tests = if run_only_passing_tests {
        get_passing_tests()
    } else {
        get_all_tests()
    };

    if let Err(_) = run_tests(tests.iter(), summarize) {
        println!("Failed.");
        std::process::exit(1);
    }

    println!("Success.");
}

fn get_passing_tests() -> std::collections::BTreeMap<String, TestFn> {
    #[rustfmt::skip]
    let mut tests: Vec<(String, TestFn)> = vec![
        ("test_invalid_fd".to_string(),
            Box::new(test_invalid_fd)),
        ("test_non_existent_fd".to_string(),
            Box::new(test_non_existent_fd)),
        ("test_invalid_sock_type".to_string(),
            Box::new(test_invalid_sock_type)),
    ];

    // optionally bind to an address before listening
    let bind_addresses = [
        None,
        Some(BindAddress {
            address: libc::INADDR_LOOPBACK.to_be(),
            port: 0u16.to_be(),
        }),
        Some(BindAddress {
            address: libc::INADDR_ANY.to_be(),
            port: 0u16.to_be(),
        }),
    ];

    // tests to repeat for different socket options
    for &sock_type in [libc::SOCK_STREAM, libc::SOCK_DGRAM].iter() {
        for &flag in [0, libc::SOCK_NONBLOCK, libc::SOCK_CLOEXEC].iter() {
            for &bind in bind_addresses.iter() {
                // add details to the test names to avoid duplicates
                let append_args =
                    |s| format!("{} <type={},flag={},bind={:?}>", s, sock_type, flag, bind);

                #[rustfmt::skip]
                let more_tests: Vec<(String, TestFn)> = vec![
                    (append_args("test_zero_backlog"),
                        Box::new(move || test_zero_backlog(sock_type, flag, bind))),
                    (append_args("test_negative_backlog"),
                        Box::new(move || test_negative_backlog(sock_type, flag, bind))),
                    (append_args("test_large_backlog"),
                        Box::new(move || test_large_backlog(sock_type, flag, bind))),
                    (append_args("test_after_close"),
                        Box::new(move || test_after_close(sock_type, flag, bind))),
                ];

                tests.extend(more_tests);
            }
        }
    }

    let num_tests = tests.len();
    let tests: std::collections::BTreeMap<_, _> = tests.into_iter().collect();

    // make sure we didn't have any duplicate tests
    assert_eq!(num_tests, tests.len());

    tests
}

fn get_all_tests() -> std::collections::BTreeMap<String, TestFn> {
    #[rustfmt::skip]
    let mut tests: Vec<(String, TestFn)> = vec![
        ("test_non_socket_fd".to_string(),
            Box::new(test_non_socket_fd)),
    ];

    let bind_addresses = [
        None,
        Some(BindAddress {
            address: libc::INADDR_LOOPBACK.to_be(),
            port: 0u16.to_be(),
        }),
        Some(BindAddress {
            address: libc::INADDR_ANY.to_be(),
            port: 0u16.to_be(),
        }),
    ];

    // tests to repeat for different socket options
    for &sock_type in [libc::SOCK_STREAM, libc::SOCK_DGRAM].iter() {
        for &flag in [0, libc::SOCK_NONBLOCK, libc::SOCK_CLOEXEC].iter() {
            for &bind in bind_addresses.iter() {
                // add details to the test names to avoid duplicates
                let append_args =
                    |s| format!("{} <type={},flag={},bind={:?}>", s, sock_type, flag, bind);

                #[rustfmt::skip]
                let more_tests: Vec<(String, TestFn)> = vec![
                    (append_args("test_listen_twice"),
                        Box::new(move || test_listen_twice(sock_type, flag, bind))),
                ];

                tests.extend(more_tests);
            }
        }
    }

    let num_tests = tests.len();
    let mut tests: std::collections::BTreeMap<_, _> = tests.into_iter().collect();

    // make sure we didn't have any duplicate tests
    assert_eq!(num_tests, tests.len());

    // add all of the passing tests
    tests.extend(get_passing_tests());

    tests
}

fn run_tests<'a, I>(tests: I, summarize: bool) -> Result<(), ()>
where
    I: Iterator<Item = (&'a String, &'a TestFn)>,
{
    for (test_name, test_fn) in tests {
        print!("Testing {}...", test_name);

        match test_fn() {
            Err(msg) => {
                println!(" ✗ ({})", msg);
                if !summarize {
                    return Err(());
                }
            }
            Ok(_) => {
                println!(" ✓");
            }
        }
    }

    Ok(())
}

/// Test listen using an argument that cannot be a fd.
fn test_invalid_fd() -> Result<(), String> {
    let args = ListenArguments { fd: -1, backlog: 0 };

    check_listen_call(&args, Some(libc::EBADF))
}

/// Test listen using an argument that could be a fd, but is not.
fn test_non_existent_fd() -> Result<(), String> {
    let args = ListenArguments {
        fd: 8934,
        backlog: 0,
    };

    check_listen_call(&args, Some(libc::EBADF))
}

/// Test listen using a valid fd that is not a socket.
fn test_non_socket_fd() -> Result<(), String> {
    let args = ListenArguments {
        fd: 0, // assume the fd 0 is already open and is not a socket
        backlog: 0,
    };

    check_listen_call(&args, Some(libc::ENOTSOCK))
}

/// Test listen using an invalid socket type.
fn test_invalid_sock_type() -> Result<(), String> {
    let fd = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0) };
    assert!(fd >= 0);

    let args = ListenArguments { fd: fd, backlog: 0 };

    run_and_close_fds(&[fd], || check_listen_call(&args, Some(libc::EOPNOTSUPP)))
}

/// Test listen using a backlog of 0.
fn test_zero_backlog(
    sock_type: libc::c_int,
    flag: libc::c_int,
    bind: Option<BindAddress>,
) -> Result<(), String> {
    let fd = unsafe { libc::socket(libc::AF_INET, sock_type | flag, 0) };
    assert!(fd >= 0);

    if let Some(address) = bind {
        bind_fd(fd, address);
    }

    let args = ListenArguments { fd: fd, backlog: 0 };

    let expected_errno = if [libc::SOCK_STREAM, libc::SOCK_SEQPACKET].contains(&sock_type) {
        None
    } else {
        Some(libc::EOPNOTSUPP)
    };

    run_and_close_fds(&[fd], || check_listen_call(&args, expected_errno))
}

/// Test listen using a backlog of -1.
fn test_negative_backlog(
    sock_type: libc::c_int,
    flag: libc::c_int,
    bind: Option<BindAddress>,
) -> Result<(), String> {
    let fd = unsafe { libc::socket(libc::AF_INET, sock_type | flag, 0) };
    assert!(fd >= 0);

    if let Some(address) = bind {
        bind_fd(fd, address);
    }

    let args = ListenArguments {
        fd: fd,
        backlog: -1,
    };

    let expected_errno = if [libc::SOCK_STREAM, libc::SOCK_SEQPACKET].contains(&sock_type) {
        None
    } else {
        Some(libc::EOPNOTSUPP)
    };

    run_and_close_fds(&[fd], || check_listen_call(&args, expected_errno))
}

/// Test listen using a backlog of INT_MAX.
fn test_large_backlog(
    sock_type: libc::c_int,
    flag: libc::c_int,
    bind: Option<BindAddress>,
) -> Result<(), String> {
    let fd = unsafe { libc::socket(libc::AF_INET, sock_type | flag, 0) };
    assert!(fd >= 0);

    if let Some(address) = bind {
        bind_fd(fd, address);
    }

    let args = ListenArguments {
        fd: fd,
        backlog: libc::INT_MAX,
    };

    let expected_errno = if [libc::SOCK_STREAM, libc::SOCK_SEQPACKET].contains(&sock_type) {
        None
    } else {
        Some(libc::EOPNOTSUPP)
    };

    run_and_close_fds(&[fd], || check_listen_call(&args, expected_errno))
}

/// Test calling listen twice for the same socket.
fn test_listen_twice(
    sock_type: libc::c_int,
    flag: libc::c_int,
    bind: Option<BindAddress>,
) -> Result<(), String> {
    let fd = unsafe { libc::socket(libc::AF_INET, sock_type | flag, 0) };
    assert!(fd >= 0);

    if let Some(address) = bind {
        bind_fd(fd, address);
    }

    let args1 = ListenArguments {
        fd: fd,
        backlog: 10,
    };

    let args2 = ListenArguments { fd: fd, backlog: 0 };

    let expected_errno = if [libc::SOCK_STREAM, libc::SOCK_SEQPACKET].contains(&sock_type) {
        None
    } else {
        Some(libc::EOPNOTSUPP)
    };

    run_and_close_fds(&[fd], || {
        check_listen_call(&args1, expected_errno)?;
        check_listen_call(&args2, expected_errno)
    })
}

/// Test listen after closing the socket.
fn test_after_close(
    sock_type: libc::c_int,
    flag: libc::c_int,
    bind: Option<BindAddress>,
) -> Result<(), String> {
    let fd = unsafe { libc::socket(libc::AF_INET, sock_type | flag, 0) };
    assert!(fd >= 0);

    if let Some(address) = bind {
        bind_fd(fd, address);
    }

    // close the file descriptor
    run_and_close_fds(&[fd], || Ok(())).unwrap();

    let args = ListenArguments {
        fd: fd,
        backlog: 100,
    };

    check_listen_call(&args, Some(libc::EBADF))
}

/// Bind the fd to the address.
fn bind_fd(fd: libc::c_int, bind: BindAddress) {
    let addr = libc::sockaddr_in {
        sin_family: libc::AF_INET as u16,
        sin_port: bind.port,
        sin_addr: libc::in_addr {
            s_addr: bind.address,
        },
        sin_zero: [0; 8],
    };
    let rv = unsafe {
        libc::bind(
            fd,
            &addr as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of_val(&addr) as u32,
        )
    };
    assert_eq!(rv, 0);
}

/// Run the function and then close any given file descriptors, even if there was an error.
fn run_and_close_fds<F>(fds: &[libc::c_int], f: F) -> Result<(), String>
where
    F: Fn() -> Result<(), String>,
{
    let rv = f();

    for fd in fds.iter() {
        let fd = *fd;
        let rv_close = unsafe { libc::close(fd) };
        assert_eq!(rv_close, 0, "Could not close the fd");
    }

    rv
}

fn get_errno() -> i32 {
    std::io::Error::last_os_error().raw_os_error().unwrap()
}

fn get_errno_message(errno: i32) -> String {
    let cstr;
    unsafe {
        let error_ptr = libc::strerror(errno);
        cstr = std::ffi::CStr::from_ptr(error_ptr)
    }
    cstr.to_string_lossy().into_owned()
}

fn check_listen_call(
    args: &ListenArguments,
    expected_errno: Option<libc::c_int>,
) -> Result<(), String> {
    let rv = unsafe { libc::listen(args.fd, args.backlog) };

    let errno = get_errno();

    match expected_errno {
        // if we expect the socket() call to return an error (rv should be -1)
        Some(expected_errno) => {
            if rv != -1 {
                return Err(format!("Expecting a return value of -1, received {}", rv));
            }
            if errno != expected_errno {
                return Err(format!(
                    "Expecting errno {} \"{}\", received {} \"{}\"",
                    expected_errno,
                    get_errno_message(expected_errno),
                    errno,
                    get_errno_message(errno)
                ));
            }
        }
        // if no error is expected (rv should be 0)
        None => {
            if rv != 0 {
                return Err(format!(
                    "Expecting a return value of 0, received {} \"{}\"",
                    rv,
                    get_errno_message(errno)
                ));
            }
        }
    }

    Ok(())
}
