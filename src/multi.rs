use pb::ProgressBar;
use std::str::from_utf8;
use tty;
use std::io::{self, Stdout, Result, Write};
use std::sync::mpsc;
use std::sync::mpsc::{Sender, Receiver};

pub struct MultiBar<T: Write> {
    nlines: usize,

    lines: Vec<String>,

    nbars: usize,

    chan: (Sender<WriteMsg>, Receiver<WriteMsg>),

    handle: T,
}

impl MultiBar<Stdout> {
    /// Create a new MultiBar with stdout as a writer.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// use std::thread;
    /// use pbr::MultiBar;
    ///
    /// let mut mb = MultiBar::new();
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
    /// // start listen to all bars changes.
    /// // this is a blocking operation, until all bars will finish.
    /// // to ignore blocking, you can run it in a different thread.
    /// mb.listen();
    /// ```
    pub fn new() -> MultiBar<Stdout> {
        MultiBar::on(::std::io::stdout())
    }
}

impl<T: Write> MultiBar<T> {
    /// Create a new MultiBar with an arbitrary writer.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use pbr::MultiBar;
    /// use std::io::stderr;
    ///
    /// let mut mb = MultiBar::on(stderr());
    /// // ...
    /// // see full example in `MultiBar::new`
    /// // ...
    /// ```
    pub fn on(handle: T) -> MultiBar<T> {
        MultiBar {
            nlines: 0,
            nbars: 0,
            lines: Vec::new(),
            chan: mpsc::channel(),
            handle: handle,
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
    /// let mut mb = MultiBar::new();
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
    /// mb.listen();
    /// ```
    pub fn println(&mut self, s: &str) {
        self.lines.push(s.to_owned());
        self.nlines += 1;
    }

    /// create_bar creates new `ProgressBar` with `Pipe` as the writer.
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
    /// let mut mb = MultiBar::new();
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
    /// mb.listen();
    /// ```
    pub fn create_bar(&mut self, total: u64) -> ProgressBar<Pipe> {
        self.println("");
        self.nbars += 1;
        let mut p = ProgressBar::on(Pipe {
                                        level: self.nlines - 1,
                                        chan: self.chan.0.clone(),
                                    },
                                    total);
        p.is_multibar = true;
        p.add(0);
        p
    }

    pub fn create_log_target(&mut self) -> LogTarget {
        LogTarget{
            buf: Vec::new(),
            chan: self.chan.0.clone(),
        }
    }

    /// listen start listen to all bars changes.
    ///
    /// `ProgressBar` that finish its work, must call `finish()` (or `finish_print`)
    /// to notify the `MultiBar` about it.
    ///
    /// This is a blocking operation and blocks until all bars will
    /// finish.
    /// To ignore blocking, you can run it in a different thread.
    ///
    /// # Examples
    ///
    /// ```
    /// # extern crate pbr;
    /// # use std::thread;
    /// # fn main() {
    /// use ::pbr::MultiBar;
    ///
    /// let mut mb = MultiBar::new();
    ///
    /// // ...
    /// // create some bars here
    /// // ...
    ///
    /// thread::spawn(move || {
    ///     mb.listen();
    ///     println!("all bars done!");
    /// });
    ///
    /// // ...
    /// # }
    /// ```
    pub fn listen(&mut self) {
        let mut first = true;
        let mut nbars = self.nbars;
        while nbars > 0 {
            let mut log_line = None;

            // receive message
            let msg = self.chan.1.recv().unwrap();
            match msg {
                WriteMsg::ProgressUpdate{level,line} => {
                    self.lines[level] = line;
                },
                WriteMsg::ProgressClear{level,line} => {
                    self.lines[level] = line;
                    nbars -= 1;
                },
                WriteMsg::ProgressFinish{level,line} => {
                    // writing lines below progress not supported; treat
                    // as log message
                    let _ = level;
                    nbars -= 1;
                    if line.is_empty() { continue; }
                    log_line = Some(line);
                },
                WriteMsg::Log{line} => {
                    if line.is_empty() { continue; }
                    log_line = Some(line);
                },
            }

            // and draw
            let mut out = String::new();
            if !first {
                out += &tty::move_cursor_up(self.nlines);
            } else {
                first = false;
            }
            if let Some(line) = log_line {
                out += "\r";
                out += &tty::clear_after_cursor();
                out += &line;
                out += "\n";
            }
            for l in self.lines.iter() {
                out += "\r";
                out += &l;
                out += "\n";
            }
            printfl!(self.handle, "{}", out);
        }
    }
}

pub struct Pipe {
    level: usize,
    chan: Sender<WriteMsg>,
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> Result<usize> {
        let s = from_utf8(buf).unwrap().to_owned();
        if s == "" {
            self.chan.send(WriteMsg::ProgressFinish{
                level: self.level,
                line: s,
            })
            .unwrap();
        } else {
            self.chan.send(WriteMsg::ProgressUpdate{
                level: self.level,
                line: s,
            })
            .unwrap();
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> Result<()> {
        Ok(())
    }
}

pub struct LogTarget {
    buf: Vec<u8>,
    chan: Sender<WriteMsg>,
}

impl Write for LogTarget {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        use std::mem::replace;

        self.buf.extend_from_slice(buf);
        // find last newline and flush the part before it
        for pos in (0..self.buf.len()).rev() {
            if self.buf[pos] == b'\n' {
                let rem = self.buf.split_off(pos+1);
                let msg = replace(&mut self.buf, rem);
                self.chan.send(WriteMsg::Log{
                    line: from_utf8(&msg[..pos]).unwrap().to_owned(),
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
        self.chan.send(WriteMsg::Log{
            line: from_utf8(&msg).unwrap().to_owned(),
        })
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(())
    }
}

// WriteMsg is the message format used to communicate
// between MultiBar and its bars
enum WriteMsg {
    ProgressUpdate {
        level: usize,
        line: String,
    },
    ProgressClear {
        level: usize,
        line: String,
    },
    ProgressFinish {
        level: usize,
        line: String,
    },
    Log {
        line: String,
    },
}
