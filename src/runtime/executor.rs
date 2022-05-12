use core::future::Future;
use std::{collections::HashMap, task::Context};

use nix::{
    errno::Errno,
    sys::epoll::{self, EpollCreateFlags, EpollEvent, EpollOp},
};

use super::rt::{EMPTY, EPFD, FD_MAP, NEW_TASK_STACK, TASK_ARRAY, TASK_FD_OP};

/// 协程调度器
pub struct Executor<'a> {
    // fd = idx(任务所在数组的索引)
    events: &'a mut [EpollEvent],
}

impl<'a> Executor<'a> {
    pub fn new(events: &'a mut [EpollEvent]) -> Self {
        Self { events }
    }
    pub fn run(&mut self) {
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        // FD_MAP初始化的值在这里，不要放到unsafe代码块里了，不然代码块结束就直接析构了，后面运行会段错误
        let mut global_fd_map = HashMap::new();

        unsafe {
            // 初始化EPFD
            EPFD = match epoll::epoll_create1(EpollCreateFlags::EPOLL_CLOEXEC) {
                Ok(it) => it,
                Err(err) => {
                    panic!("{}", err.desc());
                }
            };

            // 初始化FD_MAP
            FD_MAP = &mut global_fd_map;
        }

        loop {
            // poll所有新的协程，第一次运行
            self.poll_new_task(&mut cx);

            let n = match epoll::epoll_wait(unsafe { EPFD }, &mut self.events, -1) {
                Ok(n) => n,
                Err(err) => {
                    // 错误码为EINTR我们直接重试即可
                    if err == Errno::EINTR {
                        continue;
                    } else {
                        panic!("{}", err.desc());
                    }
                }
            };

            unsafe {
                for i in 0..n {
                    let fd = self.events[i].data() as i32;
                    let (idx, ep_flag) = (*FD_MAP)[&fd].clone();

                    // 当前协程运行完成
                    if TASK_ARRAY[idx].as_mut().poll(&mut cx).is_ready() {
                        EMPTY.push(idx);
                        continue;
                    }

                    // 运行到这里说明协程还没有运行完成，协程只是运行到下一步还没有完成，我们要判断是否有新的IO产生

                    // fd与全局fd记录的不一致，说明上面的poll中产生了新的IO
                    if fd != TASK_FD_OP.0 {
                        (*FD_MAP).insert(fd, (idx, TASK_FD_OP.1));

                        let mut event = EpollEvent::new(TASK_FD_OP.1, fd as u64);
                        if let Err(err) =
                            epoll::epoll_ctl(EPFD, EpollOp::EpollCtlAdd, TASK_FD_OP.0, &mut event)
                        {
                            panic!("{}", err.desc());
                        }
                    }
                    if ep_flag != TASK_FD_OP.1 {
                        /*
                            todo 这里改成了elif感觉elif才对
                            只要fd发生变化，事件是什么不重要了，我只需要将新的fd与对应事件加入epol即可
                            如果fd没变，事件变了，我们只需要更改新的事件即可
                        */
                        let mut event = EpollEvent::new(TASK_FD_OP.1, TASK_FD_OP.0 as u64);
                        match epoll::epoll_ctl(EPFD, EpollOp::EpollCtlMod, TASK_FD_OP.0, &mut event)
                        {
                            Ok(_) => (*FD_MAP).get_mut(&fd).unwrap().1 = TASK_FD_OP.1,
                            Err(err) => panic!("{}", err.desc()),
                        };
                    }
                }
            }
        }
    }

    fn poll_new_task(&mut self, cx: &mut Context) {
        unsafe {
            while let Some(idx) = NEW_TASK_STACK.pop() {
                let task = &mut TASK_ARRAY[idx];
                // 直接poll
                if task.as_mut().poll(cx).is_pending() {
                    // 如果是pending就加入(*FD_MAP)中
                    (*FD_MAP).insert(TASK_FD_OP.0, (idx, TASK_FD_OP.1));
                    // 创建epoll_event结构
                    let mut event = EpollEvent::new(TASK_FD_OP.1, TASK_FD_OP.0 as u64);
                    if let Err(err) =
                        epoll::epoll_ctl(EPFD, EpollOp::EpollCtlAdd, TASK_FD_OP.0, &mut event)
                    {
                        panic!("{}", err.desc());
                    }
                } else {
                    EMPTY.push(idx);
                }
            }
        }
    }
}

/// 分配协程
pub fn spawn<F>(future: F)
where
    F: Future<Output = ()> + 'static,
{
    unsafe {
        // 如果empty中有值，就将新的协程放到空位上
        if let Some(idx) = EMPTY.pop() {
            TASK_ARRAY[idx] = Box::pin(future);
            NEW_TASK_STACK.push(idx);
        } else {
            TASK_ARRAY.push(Box::pin(future));
            NEW_TASK_STACK.push(TASK_ARRAY.len() - 1);
        }
    }
}
