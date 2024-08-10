use std::io::Write;
use std::ops::{Fn, FnOnce, FnMut};

use log::{error, trace};

pub struct App {

}

pub trait AppTransport {
    fn write_stdout(&self, buf: &[u8], num: usize);
    fn write_pty(&self, buf: &[u8], num: usize);
}

#[derive(Debug)]
pub enum AppError {
    
}

impl App {
    pub fn new() -> Result<Self, AppError> {
        Ok(Self {
        })
    }

    pub fn ptyin<'b, FSTD: Fn(&'b [u8], usize), FPTY: Fn(&'b [u8], usize)>(
        &self,
        buf: &'b [u8],
        num: usize,
        write_stdout: FSTD,
        _write_ptyout: FPTY,
    ) {
        write_stdout(&buf, num);
    }

    pub fn stdin<'b, FSTD: Fn(&'b [u8], usize), FPTY: Fn(&'b [u8], usize)>(
        &self,
        buf: &'b [u8],
        num: usize,
        _write_stdout: FSTD,
        write_ptyout: FPTY,
    ) {
        write_ptyout(&buf, num);
    }
}