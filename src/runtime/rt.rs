use core::pin::Pin;
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
};

use lazy_static::lazy_static;
use nix::sys::epoll::{self, EpollFlags};

// 协程任务类型
pub type Task = Pin<Box<dyn Future<Output = ()>>>;

// 存储任务的数组
pub static mut TASK_ARRAY: Vec<Task> = Vec::new();

// 记录已完成的任务索引
pub static mut EMPTY: Vec<usize> = Vec::new();

// 记录还未被poll的任务
pub static mut NEW_TASK_STACK: Vec<usize> = Vec::new();

// 记录上一次epoll通知的fd与事件类型
// (fd, event)
pub static mut TASK_FD_OP: (i32, epoll::EpollFlags) = (0, epoll::EpollFlags::empty());

pub static mut EPFD: i32 = -1;

lazy_static! {
    pub static ref FD_MAP: Arc<Mutex<HashMap<i32, (usize, EpollFlags)>>> =
        Arc::new(Mutex::new(HashMap::new()));
}
