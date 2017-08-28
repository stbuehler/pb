use pb::ProgressBar;
use tty;
use std::io::{self, Stdout, Write};
use std::sync::mpsc;
use std::sync::mpsc::{Sender, Receiver};
use std::sync::{Arc,Mutex};

struct SharedState {
    nlines: usize,
}

impl SharedState {
    fn new_level(&mut self) -> usize {
        let level = self.nlines;
        self.nlines += 1;
        level
    }
}

pub struct MultiBar {
    shared: Arc<Mutex<SharedState>>,
    send: Sender<WriteMsg>,
}

impl MultiBar {
    /// Create a new MultiBar with stdout as a writer.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::thread;
    /// use pbr::MultiBar;
    ///
    /// let (mb, mb_listener) = MultiBar::new();
    /// mb.println("Application header:");
    ///
    /// let mut p1 = mb.create_bar(count);
    /// let _ = thread::spawn(move || {
    ///     for _ in 0..count {
    ///         p1.inc();
    ///         thread::sleep(Duration::from_millis(100));
    ///     }
    ///     // notify the multibar that this bar finished.
    ///     p1.finish();
    /// });
    ///
    /// mb.println("add a separator between the two bars");
    ///
    /// let mut p2 = mb.create_bar(count * 2);
    /// let _ = thread::spawn(move || {
    ///     for _ in 0..count * 2 {
    ///         p2.inc();
    ///         thread::sleep(Duration::from_millis(100));
    ///     }
    ///     // notify the multibar that this bar finished.
    ///     p2.finish();
    /// });
    ///
    /// // all bars are created, not needed anymore. would block `listen`
    /// // otherwise.
    /// drop(mb);
    ///
    /// // start listen to all bars changes.
    /// // this is a blocking operation, until all bars will finish.
    /// // to ignore blocking, you can run it in a different thread.
    /// mb_listener.listen();
    /// ```
    pub fn new() -> (MultiBar, MultiBarListener<Stdout>) {
        MultiBar::on(::std::io::stdout())
    }

    /// Create a new MultiBar with an arbitrary writer.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use pbr::MultiBar;
    /// use std::io::stderr;
    ///
    /// let (mb, mb_listener) = MultiBar::on(stderr());
    /// // ...
    /// // see full example in `MultiBar::new`
    /// // ...
    /// ```
    pub fn on<T: Write>(handle: T) -> (MultiBar, MultiBarListener<T>) {
        let shared = Arc::new(Mutex::new(SharedState{
            nlines: 0,
        }));
        let (send, recv) = mpsc::channel();
        (
            MultiBar {
                shared: shared,
                send: send,
            },
            MultiBarListener{
                recv: recv,
                lines: Vec::new(),
                handle: handle,
            },
        )
    }

    fn new_line(&self) -> MultiBarLine {
        let level = self.shared.lock().unwrap().new_level();
        MultiBarLine{
            level: level,
            send: Some(self.send.clone()),
            shared: self.shared.clone(),
        }
    }

    /// println used to add text lines between the bars.
    /// for example: you could add a header to your application,
    /// or text separators between bars.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use pbr::MultiBar;
    ///
    /// let (mb, mb_listener) = MultiBar::new();
    /// mb.println("Application header:");
    ///
    /// let mut p1 = MultiBar::create_bar(count);
    /// // ...
    ///
    /// mb.println("Text line between bar1 and bar2");
    ///
    /// let mut p2 = MultiBar::create_bar(count);
    /// // ...
    ///
    /// mb.println("Text line between bar2 and bar3");
    ///
    /// // ...
    /// // ...
    /// mb_listener.listen();
    /// ```
    pub fn println(&self, s: &str) -> MultiBarLine {
        let mut line = self.new_line();
        line.update_line(s);
        line
    }

    /// create_bar creates new `ProgressBar` with `MultiBarLine` as the writer.
    ///
    /// The ordering of the method calls is important. it means that in
    /// the first call, you get a progress bar in level 1, in the 2nd call,
    /// you get a progress bar in level 2, and so on.
    ///
    /// ProgressBar that finish its work, must call `finish()` (or `finish_print`)
    /// to notify the `MultiBar` about it.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use pbr::MultiBar;
    ///
    /// let (mb, mb_listener) = MultiBar::new();
    ///
    /// // progress bar in level 1
    /// let mut p1 = MultiBar::create_bar(count1);
    /// // ...
    ///
    /// // progress bar in level 2
    /// let mut p2 = MultiBar::create_bar(count2);
    /// // ...
    ///
    /// // progress bar in level 3
    /// let mut p3 = MultiBar::create_bar(count3);
    ///
    /// // ...
    /// drop(mb);
    /// mb_listener.listen();
    /// ```
    pub fn create_bar(&self, total: u64) -> ProgressBar<MultiBarLine> {
        let line = self.new_line();

        let mut p = ProgressBar::on(
            line,
            total,
        );
        p.tick();
        p
    }

    pub fn create_log_target(&self) -> LogTarget {
        LogTarget {
            buf: Vec::new(),
            send: self.send.clone(),
        }
    }
}


pub struct MultiBarListener<T: Write> {
    recv: Receiver<WriteMsg>,
    lines: Vec<Option<String>>,
    handle: T,
}

enum ParsedMessage {
    NoChanges,
    #[allow(dead_code)]
    Changed{level: usize},
    Log{data: Vec<u8>},
    Refresh,
}

impl<T: Write> MultiBarListener<T> {
    /// start listen to line (progress bar) changes.
    ///
    /// This blocks until all lines and bars are finished or dropped;
    /// the original `MultiBar` needs to be dropped to (as otherwise
    /// new lines and bars could be created).
    ///
    /// Also waits for any attached `LogTarget` to get dropped.
    ///
    /// Can be run in a separate thread.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate pbr;
    /// # use std::thread;
    /// # fn main() {
    /// use ::pbr::MultiBar;
    ///
    /// let (mb, mb_listener) = MultiBar::new();
    ///
    /// // ...
    /// // create some bars here
    /// // ...
    ///
    /// thread::spawn(move || {
    ///     mb_listener.listen();
    ///     println!("all bars done!");
    /// });
    ///
    /// // ...
    /// drop(mb);
    /// # }
    /// ```
    pub fn listen(mut self) {
        let mut previous_lines = 0;

        // warmup
        loop {
            // receive message
            let msg = match self.recv.try_recv() {
                Ok(msg) => msg,
                Err(_) => break,
            };
            match self.parse_message(msg) {
                ParsedMessage::NoChanges => continue,
                ParsedMessage::Changed{..} => (),
                ParsedMessage::Log{data} => {
                    self.handle.write_all(&data).unwrap();
                    self.handle.flush().unwrap();
                },
                ParsedMessage::Refresh => (),
            }
        }

        // initial draw
        previous_lines = self.redraw(previous_lines, None);

        loop {
            // receive message
            let msg = match self.recv.recv() {
                Ok(msg) => msg,
                // only fails if there are no senders - which is just
                // what we waited for
                Err(_) => return,
            };
            let log_line = match self.parse_message(msg) {
                ParsedMessage::NoChanges => continue,
                ParsedMessage::Changed{..} => None,
                ParsedMessage::Log{data} => Some(data),
                ParsedMessage::Refresh => None,
            };

            previous_lines = self.redraw(previous_lines, log_line);
        }
    }

    // returns number of drawn lines
    fn redraw(&mut self, previous_lines: usize, log_data: Option<Vec<u8>>) -> usize {
        // and draw
        let mut out = Vec::<u8>::new();
        let append = |out: &mut Vec<u8>, s: &str| {
            out.extend_from_slice(s.as_bytes());
        };
        let append_raw = |out: &mut Vec<u8>, s: &[u8]| {
            out.extend_from_slice(s);
        };
        if previous_lines > 0 {
            append(&mut out, &tty::move_cursor_up(previous_lines));
        }

        let clear_until_newline = tty::clear_until_newline();

        if let Some(log_data) = log_data {
            append(&mut out, "\r");
            append(&mut out, &tty::clear_after_cursor());
            append_raw(&mut out, &log_data);
            append(&mut out, "\n");
        }
        let mut current_lines = 0;
        for l in self.lines.iter() {
            if let Some(ref l) = *l {
                current_lines += 1;
                append(&mut out, "\r");
                append(&mut out, &l);
                append(&mut out, &clear_until_newline);
                append(&mut out, "\n");
            }
        }
        self.handle.write_all(&out).unwrap();
        self.handle.flush().unwrap();

        current_lines
    }

    fn parse_message(&mut self, msg: WriteMsg) -> ParsedMessage {
        match msg {
            WriteMsg::UpdateLine{level,line} => {
                if level >= self.lines.len() {
                    self.lines.resize(level + 1, None);
                    self.lines[level] = Some(line);
                    // wasn't there before, refresh
                    ParsedMessage::Refresh
                } else if self.lines[level].is_none() {
                    self.lines[level] = Some(line);
                    // wasn't there before, refresh
                    ParsedMessage::Refresh
                } else {
                    self.lines[level] = Some(line);
                    // just an update, could be optizimed
                    ParsedMessage::Changed{level}
                }
            },
            WriteMsg::RemoveLine{level} => {
                self.lines[level] = None;
                ParsedMessage::Refresh
            },
            WriteMsg::Log{data} => {
                if data.is_empty() {
                    ParsedMessage::NoChanges
                } else {
                    ParsedMessage::Log{data}
                }
            },
        }
    }
}

pub struct MultiBarLine {
    level: usize,
    send: Option<Sender<WriteMsg>>,
    shared: Arc<Mutex<SharedState>>,
}

impl MultiBarLine {
    pub fn new_line(&self) -> MultiBarLine {
        let level = self.shared.lock().unwrap().new_level();
        MultiBarLine{
            level: level,
            send: self.send.clone(),
            shared: self.shared.clone(),
        }
    }

    fn send(&mut self, m: WriteMsg) {
        self.send.as_mut().unwrap().send(m).unwrap();
    }

    pub fn update_line(&mut self, line: &str) {
        let m = WriteMsg::UpdateLine{
            level: self.level,
            line: line.to_string(),
        };
        self.send(m);
    }

    pub fn remove_line(&mut self) {
        let m = WriteMsg::RemoveLine{
            level: self.level,
        };
        self.send(m);
    }

    pub fn log(&mut self, message: &str) {
        let m = WriteMsg::Log{
            data: message.as_bytes().to_owned(),
        };
        self.send(m);
    }

    pub fn finish(&mut self) {
        self.send.take();
    }
}


pub struct LogTarget {
    buf: Vec<u8>,
    send: Sender<WriteMsg>,
}

impl Write for LogTarget {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        use std::mem::replace;

        self.buf.extend_from_slice(buf);
        // find last newline and flush the part before it
        for pos in (0..self.buf.len()).rev() {
            if self.buf[pos] == b'\n' {
                let rem = self.buf.split_off(pos+1);
                let mut msg = replace(&mut self.buf, rem);
                msg.truncate(pos);
                self.send.send(WriteMsg::Log{
                    data: msg,
                })
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
                break;
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        use std::mem::replace;

        let msg = replace(&mut self.buf, Vec::new());
        self.send.send(WriteMsg::Log{
            data: msg,
        })
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(())
    }
}

impl ::private::SealedProgressReceiver for MultiBarLine {
    fn update_progress(&mut self, line: &str) {
        self.update_line(line);
    }

    fn clear_progress(&mut self, line: &str) {
        self.update_line(line);
    }

    fn finish_with(&mut self, line: &str) {
        self.log(line);
    }
}

impl ::ProgressReceiver for MultiBarLine {
}

// WriteMsg is the message format used to communicate
// between MultiBar and its bars
enum WriteMsg {
    UpdateLine {
        level: usize,
        line: String,
    },
    RemoveLine {
        level: usize,
    },
    Log {
        data: Vec<u8>,
    },
}
