use errors::RescResult;
use redis::{self, Commands, Connection};
use rules::Ruleset;
/// A watcher watches the events incoming in one specific queue
/// and applies rules to generate tasks
use std::time::SystemTime;

#[derive(Debug)]
pub struct Watcher {
    pub redis_url: String,
    pub input_queue: String,
    pub taken_queue: String,
    pub ruleset: Ruleset,
}

impl Watcher {
    fn watch_input_queue(&self, con: &Connection) -> RescResult<()> {
        info!("watcher launched on queue {:?}...", &self.input_queue);
        while let Ok(done) = con.brpoplpush::<_, String>(&self.input_queue, &self.taken_queue, 0) {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let now = now as f64; // fine with a timestamp in seconds because < 2^51
            info!(
                "<- got {:?} in queue {:?} @ {}",
                &done, &self.input_queue, now
            );
            let matching_rules = self.ruleset.matching_rules(&done);
            debug!(" {} matching rule(s)", matching_rules.len());
            for r in &matching_rules {
                debug!(" applying rule {:?}", r.name);
                match r.results(&done) {
                    Ok(results) => {
                        for r in &results {
                            if let Ok(time) = con.zscore::<_, _, i32>(&r.set, &r.task) {
                                info!("  task {:?} already queued @ {}", &r.task, time);
                                continue;
                            }
                            info!(
                                "  ->  {:?} pushed to queue {:?} and set {:?}",
                                &r.task, &r.queue, &r.set
                            );
                            con.lpush::<_, _, i32>(&r.queue, &r.task)?;
                            con.zadd::<_, f64, _, i32>(&r.set, &r.task, now)?;
                        }
                    }
                    Err(err) => error!("  Rule execution failed: {:?}", err),
                }
            }
            con.lrem(&self.taken_queue, 1, &done)?;
        }
        Ok(()) // unreachable but necessary for signature (and might be reached in the future)
    }

    pub fn run(&self) {
        let client = redis::Client::open(&*self.redis_url).unwrap();
        let con = client.get_connection().unwrap();
        debug!("got redis connection");
        match self.watch_input_queue(&con) {
            Ok(_) => error!("Watcher unexpectedly finished"),
            Err(e) => error!("Watcher crashed: {:?}", e),
        }
    }
}
