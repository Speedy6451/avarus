use tracing::{info, trace, instrument};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;
use tokio::task::{JoinHandle, AbortHandle};

use crate::names::Name;
use crate::{turtle::TurtleCommander, blocks::Position};

pub enum TaskState {
    Ready(Position),
    Waiting,
    Complete,
}

#[typetag::serde(tag = "task")]
pub trait Task: Send + Sync {
    /// Execute the task
    fn run(&mut self, turtle: TurtleCommander) -> AbortHandle;
    /// Return Some if the task should be scheduled
    fn poll(&mut self) -> TaskState;
}

#[derive(Serialize, Deserialize)]
pub struct Scheduler {
    #[serde(skip)]
    turtles: Vec<(TurtleCommander, Option<AbortHandle>)>,
    tasks: Vec<Box<dyn Task>>,
    #[serde(skip)]
    shutdown: Option<oneshot::Sender<()>>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            turtles: Vec::new(),
            tasks: Vec::new(),
            shutdown:None,
        }
    }
}

impl Scheduler {
    /// Add a new turtle to the scheduler
    pub fn add_turtle(&mut self, turtle: &TurtleCommander) {
        let name = turtle.name();
        if self.turtles.iter().any(|(t,_)| t.name() == name ) {
            return;
        }
        info!("registered {}", name.to_owned().to_str());
        self.turtles.push((
                turtle.clone(),
                None
        ));
    }

    pub fn add_task(&mut self, task: Box<dyn Task>) {
        trace!("new {} task", task.typetag_name());
        self.tasks.push(task);
    }

    pub async fn poll(&mut self) {
        for turtle in &mut self.turtles {
            if let Some(join)  = &turtle.1 {
                if join.is_finished() {
                    trace!("#{} completed task", turtle.0.name().to_num());
                    turtle.1 = None;
                }
            }
        }

        if self.shutdown.is_some() {
            trace!("checking remaining tasks");
            if !self.turtles.iter().any(|t| t.1.is_some()) {
                trace!("all tasks complete");
                self.shutdown.take().unwrap().send(()).unwrap();
            }

            for turtle in self.turtles.iter().filter(|t| t.1.is_some()) {
                trace!("waiting on {}", turtle.0.name().to_str());
            }

            return;
        }

        let mut free_turtles: Vec<&mut (TurtleCommander, Option<AbortHandle>)> = 
            self.turtles.iter_mut().filter(|t| t.1.is_none()).collect();

        let mut turtle_positions = Vec::new();
        for turtle in &free_turtles {
            turtle_positions.push(turtle.0.pos().await);
        }

        let mut done = vec![false; self.tasks.len()];
        for (i, task) in self.tasks.iter_mut().enumerate() {
            let poll = task.poll();
            if let TaskState::Ready(position) = poll {
                let closest_turtle = match free_turtles.iter_mut().zip(turtle_positions.iter())
                    .filter(|t|t.0.1.is_none()) // Don't double-schedule
                    .min_by_key( |(_,p)| {
                    p.manhattan(position)
                }) {
                    Some(turtle) => turtle.0,
                    None => break,
                };

                trace!("scheduling {} on #{}", task.typetag_name(), closest_turtle.0.name().to_num());
                closest_turtle.1 = Some(task.run(closest_turtle.0.clone()));
            }
            if let TaskState::Complete = poll {
                done[i] = true;
            }
        }

        // this feels like a hack
        let mut i = 0;
        self.tasks.retain(|_| {
            let cont = !done[i];
            i+=1;
            cont 
        });
    }

    // TODO: make awaiting this feasible
    pub fn do_on<T>(&mut self, mut task: T, turtle: Name) -> Option<()> 
        where T: FnMut(TurtleCommander) -> AbortHandle
    {
        let turtle = self.turtles.iter_mut().filter(|t| t.0.name() == turtle).next()?;
        match turtle.1 {
            Some(_) => None,
            None => {
                trace!("new adhoc task on {}", turtle.0.name().to_str());
                turtle.1 = Some(task(turtle.0.clone()));
                Some(())
            },
        }
    }

    pub fn task_on(&mut self, mut task: Box<dyn Task>, turtle: Name) -> Option<()> {
        trace!("new {} task on {}", task.typetag_name(), turtle.clone().to_str());
        let turtle = self.turtles.iter_mut().filter(|t| t.0.name() == turtle).next()?;
        match turtle.1 {
            Some(_) => None,
            None => {
                turtle.1 = Some(task.run(turtle.0.clone()));
                Some(())
            },
        }
    }

    pub async fn cancel(&mut self, turtle: Name) -> Option<()> {
        if let Some(task) = self.turtles.iter_mut().find(|t| t.0.name() == turtle)?.1.as_ref() {
            task.abort();
            info!("aborted task for #{}", turtle.to_num());
        }
        Some(())
    }

    pub fn shutdown(&mut self) -> oneshot::Receiver<()>{
        let (send, recv) = oneshot::channel();
        self.shutdown =  Some(send);
        recv
    }
}
