use log::info;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

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
    fn run(&mut self, turtle: TurtleCommander) -> JoinHandle<()>;
    /// Return Some if the task should be scheduled
    fn poll(&mut self) -> TaskState;
}

#[derive(Serialize, Deserialize)]
pub struct Scheduler {
    #[serde(skip)]
    turtles: Vec<(TurtleCommander, Option<JoinHandle<()>>)>,
    tasks: Vec<Box<dyn Task>>,
}

impl Default for Scheduler {
    fn default() -> Self {
        Self {
            turtles: Vec::new(),
            tasks: Vec::new(),
        }
    }
}

impl Scheduler {
    /// Add a new turtle to the scheduler
    /// Whether or not the turtle is already in the scheduler is not verified
    pub fn add_turtle(&mut self, turtle: &TurtleCommander) {
        self.turtles.push((
                turtle.clone(),
                None
            ));
    }

    pub fn add_task(&mut self, task: Box<dyn Task>) {
        info!("new task");
        self.tasks.push(task);
    }

    pub async fn poll(&mut self) {
        for turtle in &mut self.turtles {
            if let Some(join)  = &turtle.1 {
                if join.is_finished() {
                    turtle.1 = None;
                }
            }
        }

        let mut free_turtles: Vec<&mut (TurtleCommander, Option<JoinHandle<()>>)> = 
            self.turtles.iter_mut().filter(|t| t.1.is_none()).collect();

        let mut turtle_positions = Vec::new();
        for turtle in &free_turtles {
            turtle_positions.push(turtle.0.pos().await);
        }

        for task in &mut self.tasks {
            let poll = task.poll();
            if let TaskState::Ready(position) = poll {
                let closest_turtle = match free_turtles.iter_mut().zip(turtle_positions.iter()).min_by_key( |(_,p)| {
                    p.manhattan(position)
                }) {
                    Some(turtle) => turtle.0,
                    None => break,
                };

                closest_turtle.1 = Some(task.run(closest_turtle.0.clone()));
            }
            if let TaskState::Complete = poll {
                // TODO: removal
            }
        }
        
    }

    pub async fn cancel(&mut self, turtle: Name) -> Option<()> {
        self.turtles.iter_mut().find(|t| t.0.name() == turtle)?.1.take()?.abort();
        Some(())
    }
}
