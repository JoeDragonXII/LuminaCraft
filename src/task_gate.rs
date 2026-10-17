//! Serialize cancellation against irreversible filesystem commits.
use std::sync::atomic::{AtomicU8,Ordering};
#[derive(Default,Debug)]pub struct CommitGate(AtomicU8);
impl CommitGate{
    pub fn request_cancel(&self)->bool{self.0.compare_exchange(0,1,Ordering::SeqCst,Ordering::SeqCst).is_ok()}
    pub fn cancelled(&self)->bool{self.0.load(Ordering::SeqCst)==1}
    pub fn committing(&self)->bool{self.0.load(Ordering::SeqCst)==2}
    pub fn begin_commit(&self)->anyhow::Result<()>{
        anyhow::ensure!(self.0.compare_exchange(0,2,Ordering::SeqCst,Ordering::SeqCst).is_ok()||self.committing(),"操作已取消，未提交文件");Ok(())
    }
}
#[cfg(test)]mod tests{
    use super::*;
    #[test]fn cancellation_and_commit_are_mutually_exclusive(){
        for _ in 0..100{let gate=std::sync::Arc::new(CommitGate::default());let other=gate.clone();let cancel=std::thread::spawn(move||other.request_cancel());let committed=gate.begin_commit().is_ok();assert_ne!(committed,cancel.join().unwrap());}
    }
}
