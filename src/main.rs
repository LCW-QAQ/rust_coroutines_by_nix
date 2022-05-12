use nix::sys::epoll::EpollEvent;
use runtime::{
    executor::{spawn, Executor},
    tcp::TcpSocket,
};

mod runtime;

const CONN_NUM: usize = 100000;

fn main() {
    let listener = TcpSocket::new();
    let mut events = [EpollEvent::empty(); CONN_NUM];
    let mut executor = Executor::new(&mut events);

    spawn(async move {
        listener.bind(8001).unwrap();
        listener.listen(CONN_NUM).unwrap();

        loop {
            println!("accept ...");
            match listener.accept().await {
                Ok(fd) => {
                    println!("accept ok");
                    spawn(async move {
                        let sock = TcpSocket::new_with_fd(fd);
                        let mut buf = [0u8; 4096];

                        loop {
                            match sock.read(&mut buf).await {
                                Ok(n) => {
                                    if n == 0 {
                                        break;
                                    }

                                    println!(
                                        "read {} bytes from client, context:\n{}",
                                        n,
                                        String::from_utf8_lossy(&buf[0..n])
                                    );
                                    let resp_str = "HTTP/1.1 200 OK\r\n\r\n
<html>
    <body>
        <h1>hello, response</h1>
    </body>
</html>";
                                    let resp_bytes = resp_str.as_bytes();
                                    for (idx, b) in resp_bytes.iter().enumerate() {
                                        buf[idx] = *b;
                                    }
                                    match sock.write(&mut buf, resp_bytes.len()).await {
                                        Ok(n) => {
                                            println!("write {} bytes to client", n);
                                        }
                                        Err(err) => eprintln!("write err: {}", err.desc()),
                                    };
                                }
                                Err(err) => eprintln!("read err: {}", err.desc()),
                            };
                            break; // 一次读写候直接结束
                        }
                    });
                }
                Err(err) => eprintln!("accept err: {}", err.desc()),
            };
        }
    });

    executor.run();
}
