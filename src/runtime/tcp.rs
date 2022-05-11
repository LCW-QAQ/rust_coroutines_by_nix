use std::{future::Future, task::Poll};

use nix::{
    errno::Errno,
    fcntl::{fcntl, FcntlArg, OFlag},
    sys::{
        epoll::{self, EpollFlags},
        socket::{self, sockopt::ReusePort, AddressFamily, SockFlag, SockType, SockaddrIn},
    },
    unistd,
};

use super::rt::{EPFD, FD_MAP, TASK_FD_OP};

#[derive(Debug)]
pub struct TcpSocket {
    fd: i32,
}

impl TcpSocket {
    pub fn new() -> Self {
        // 创建tcp流
        let fd = socket::socket(
            AddressFamily::Inet,
            SockType::Stream,
            SockFlag::empty(),
            None,
        )
        .unwrap();
        Self { fd }
    }

    pub fn new_with_fd(fd: i32) -> Self {
        Self { fd }
    }

    pub fn bind(&self, port: u16) -> Result<(), Errno> {
        socket::setsockopt(self.fd, ReusePort, &true)?;
        let addr = SockaddrIn::new(0, 0, 0, 0, port);
        socket::bind(self.fd, &addr)?;
        Ok(())
    }

    pub fn listen(&self, backlog: usize) -> Result<(), Errno> {
        socket::listen(self.fd, backlog)?;
        TcpSocket::set_nonblocking(self.fd);

        Ok(())
    }

    /// 设置非阻塞
    fn set_nonblocking(fd: i32) {
        fcntl(fd, nix::fcntl::F_SETFL(OFlag::O_NONBLOCK)).unwrap();
    }

    pub fn accept(&self) -> AsyncAccept {
        AsyncAccept::new(self.fd)
    }

    pub fn conn_async(&self, addr_tuple: (u8, u8, u8, u8), port: u16) -> Result<(), Errno> {
        let (a, b, c, d) = addr_tuple;
        let mut addr = SockaddrIn::new(a, b, c, d, port);
        socket::connect(self.fd, &mut addr)?;
        Ok(())
    }

    pub fn read<'a>(&self, buf: &'a mut [u8]) -> AsyncRead<'a> {
        AsyncRead::new(self.fd, buf)
    }

    pub fn write<'a>(&self, buf: &'a [u8], n: usize) -> AsyncWrite<'a> {
        AsyncWrite::new(self.fd, &buf[0..n])
    }
}

impl Drop for TcpSocket {
    fn drop(&mut self) {
        FD_MAP.lock().unwrap().remove(&self.fd);
        unistd::close(self.fd).unwrap();
        println!("TcpSocket droped, {:?}", self);
    }
}

pub struct AsyncAccept {
    fd: i32,
}

impl AsyncAccept {
    pub fn new(fd: i32) -> Self {
        Self { fd }
    }
}

impl Future for AsyncAccept {
    type Output = Result<i32, Errno>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let this = self.get_mut();
        // 尝试在accept上建立连接
        return match socket::accept(this.fd) {
            // 连接成功当然返回Ready
            Ok(it) => Poll::Ready(Ok(it)),
            Err(err) => {
                // 再非阻塞模式下，如果err是EAGAIN表示下一次可以继续读写，直接Pending即可
                if err == Errno::EAGAIN {
                    unsafe {
                        // 记录当前协程运行后阻塞的fd与事件类型
                        TASK_FD_OP = (this.fd, EpollFlags::EPOLLIN);
                    }
                    Poll::Pending
                } else {
                    Poll::Ready(Err(err))
                }
            }
        };
    }
}

pub struct AsyncRead<'a> {
    fd: i32,
    buf: &'a mut [u8],
    first: bool,
}

impl<'a> AsyncRead<'a> {
    pub fn new(fd: i32, buf: &'a mut [u8]) -> Self {
        return Self {
            fd,
            buf: buf,
            first: true,
        };
    }
}

impl<'a> Future for AsyncRead<'a> {
    type Output = Result<usize, Errno>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let this = self.get_mut();

        unsafe {
            // 更改全局fd与event
            TASK_FD_OP = (this.fd, EpollFlags::EPOLLIN);
        }

        // 读取事件第一次进入直接Pending，听从调度器的安排就好
        if this.first {
            this.first = false;
            return Poll::Pending;
        }

        // todo 这里不需要处理EAGAIN吗
        let n = unistd::read(this.fd, this.buf)?;

        Poll::Ready(Ok(n))
    }
}

pub struct AsyncWrite<'a> {
    fd: i32,
    buf: &'a [u8],
}

impl<'a> AsyncWrite<'a> {
    pub fn new(fd: i32, buf: &'a [u8]) -> Self {
        Self { fd, buf }
    }
}

impl<'a> Future for AsyncWrite<'a> {
    type Output = Result<usize, Errno>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let this = self.get_mut();

        // todo 这里不需要处理全局fd状态吗?

        let n = match unistd::write(this.fd, this.buf) {
            Ok(it) => it,
            Err(err) => {
                // 处理EAGAIN
                if err == Errno::EAGAIN {
                    unsafe {
                        // 处理EAGAIN，这里是写入，ET在写入并不需要特殊处理，ET在读取时注意要一次性读取完成
                        TASK_FD_OP = (this.fd, EpollFlags::EPOLLOUT | EpollFlags::EPOLLET);
                    }
                    return Poll::Pending;
                } else {
                    return Poll::Ready(Err(err));
                }
            }
        };

        return Poll::Ready(Ok(n));
    }
}
