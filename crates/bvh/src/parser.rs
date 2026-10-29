use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Channel {
    XPosition,
    YPosition,
    ZPosition,
    XRotation,
    YRotation,
    ZRotation,
}

impl Channel {
    pub(crate) fn is_position(self) -> bool {
        matches!(self, Self::XPosition | Self::YPosition | Self::ZPosition)
    }

    fn parse(name: &str) -> Option<Self> {
        Some(match name {
            "Xposition" => Self::XPosition,
            "Yposition" => Self::YPosition,
            "Zposition" => Self::ZPosition,
            "Xrotation" => Self::XRotation,
            "Yrotation" => Self::YRotation,
            "Zrotation" => Self::ZRotation,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Joint {
    pub name: String,
    pub parent: Option<usize>,
    pub offset: [f32; 3],
    pub channels: Vec<Channel>,
    pub channel_start: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Bvh {
    pub joints: Vec<Joint>,
    pub frame_time: f32,
    pub channel_count: usize,
    pub frames: Vec<Vec<f32>>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

type Result<T> = std::result::Result<T, ParseError>;

fn error<T>(line: usize, message: impl Into<String>) -> Result<T> {
    Err(ParseError {
        line,
        message: message.into(),
    })
}

struct Tokens<'a> {
    tokens: Vec<(usize, &'a str)>,
    position: usize,
    last_line: usize,
}

impl<'a> Tokens<'a> {
    fn peek(&self) -> Option<(usize, &'a str)> {
        self.tokens.get(self.position).copied()
    }

    fn next(&mut self) -> Option<(usize, &'a str)> {
        let token = self.peek();
        if token.is_some() {
            self.position += 1;
        }
        token
    }

    fn expect_next(&mut self, what: &str) -> Result<(usize, &'a str)> {
        match self.next() {
            Some(token) => Ok(token),
            None => error(
                self.last_line,
                format!("unexpected end of hierarchy, expected {what}"),
            ),
        }
    }

    fn expect(&mut self, keyword: &str) -> Result<usize> {
        let (line, token) = self.expect_next(keyword)?;
        if token == keyword {
            Ok(line)
        } else {
            error(line, format!("expected {keyword}, found {token}"))
        }
    }

    fn expect_open_brace(&mut self) -> Result<()> {
        match self.next() {
            Some((_, "{")) => Ok(()),
            Some((line, token)) => error(line, format!("expected an opening brace, found {token}")),
            None => error(
                self.last_line,
                "unbalanced braces: expected an opening brace",
            ),
        }
    }

    fn float(&mut self) -> Result<f32> {
        let (line, token) = self.expect_next("a number")?;
        match token.parse::<f32>() {
            Ok(value) if value.is_finite() => Ok(value),
            _ => error(line, format!("expected a finite number, found {token}")),
        }
    }

    fn offset(&mut self) -> Result<[f32; 3]> {
        Ok([self.float()?, self.float()?, self.float()?])
    }
}

fn split_braces(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    for word in line.split_whitespace() {
        let mut start = 0;
        for (index, ch) in word.char_indices() {
            if ch == '{' || ch == '}' {
                if start < index {
                    out.push(&word[start..index]);
                }
                out.push(&word[index..index + 1]);
                start = index + 1;
            }
        }
        if start < word.len() {
            out.push(&word[start..]);
        }
    }
    out
}

pub(crate) fn parse(text: &str) -> Result<Bvh> {
    let lines: Vec<(usize, &str)> = text
        .lines()
        .enumerate()
        .map(|(index, line)| (index + 1, line))
        .collect();

    let motion_at = lines
        .iter()
        .position(|(_, line)| line.split_whitespace().next() == Some("MOTION"));
    let hierarchy_end = motion_at.unwrap_or(lines.len());

    let mut tokens = Tokens {
        tokens: Vec::new(),
        position: 0,
        last_line: lines.last().map_or(1, |(number, _)| *number),
    };
    for (number, line) in &lines[..hierarchy_end] {
        for token in split_braces(line) {
            tokens.tokens.push((*number, token));
        }
    }
    if let Some(last) = tokens.tokens.last() {
        tokens.last_line = last.0;
    }

    tokens.expect("HIERARCHY")?;
    let mut joints = Vec::new();
    let mut channel_count = 0;
    match tokens.next() {
        Some((_, "ROOT")) => {
            parse_joint(&mut tokens, None, &mut joints, &mut channel_count)?;
        }
        Some((line, token)) => return error(line, format!("expected ROOT, found {token}")),
        None => return error(tokens.last_line, "no ROOT joint in HIERARCHY"),
    }
    match tokens.next() {
        None => {}
        Some((line, "ROOT")) => return error(line, "more than one ROOT is not supported"),
        Some((line, "}")) => return error(line, "unbalanced braces: unexpected closing brace"),
        Some((line, token)) => {
            return error(line, format!("unexpected {token} after the hierarchy"));
        }
    }

    let Some(motion_at) = motion_at else {
        return error(tokens.last_line, "missing MOTION section");
    };
    let (frame_time, frames) = parse_motion(&lines[motion_at + 1..], motion_at + 1, channel_count)?;

    Ok(Bvh {
        joints,
        frame_time,
        channel_count,
        frames,
    })
}

fn parse_joint(
    tokens: &mut Tokens<'_>,
    parent: Option<usize>,
    joints: &mut Vec<Joint>,
    channel_count: &mut usize,
) -> Result<()> {
    let (name_line, name) = tokens.expect_next("a joint name")?;
    if name == "{" || name == "}" {
        return error(name_line, "expected a joint name");
    }
    if name.contains('/') {
        return error(name_line, format!("joint name {name} contains '/'"));
    }
    if joints
        .iter()
        .any(|joint| joint.parent == parent && joint.name == name)
    {
        return error(
            name_line,
            format!("duplicate joint name {name} under the same parent"),
        );
    }
    tokens.expect_open_brace()?;

    let index = joints.len();
    joints.push(Joint {
        name: name.to_string(),
        parent,
        offset: [0.0; 3],
        channels: Vec::new(),
        channel_start: *channel_count,
    });

    loop {
        let Some((line, token)) = tokens.next() else {
            return error(
                tokens.last_line,
                format!("unbalanced braces: joint {name} is never closed"),
            );
        };
        match token {
            "OFFSET" => joints[index].offset = tokens.offset()?,
            "CHANNELS" => {
                let (count_line, count) = tokens.expect_next("a channel count")?;
                let count: usize = count
                    .parse()
                    .or_else(|_| error(count_line, format!("invalid channel count {count}")))?;
                let mut channels = Vec::new();
                for _ in 0..count {
                    let (channel_line, channel) = tokens.expect_next("a channel name")?;
                    match Channel::parse(channel) {
                        Some(channel) => channels.push(channel),
                        None => {
                            return error(channel_line, format!("unknown channel {channel}"));
                        }
                    }
                }
                *channel_count += count;
                joints[index].channels = channels;
            }
            "JOINT" => parse_joint(tokens, Some(index), joints, channel_count)?,
            "End" => {
                tokens.expect("Site")?;
                tokens.expect_open_brace()?;
                loop {
                    match tokens.next() {
                        Some((_, "OFFSET")) => {
                            tokens.offset()?;
                        }
                        Some((_, "}")) => break,
                        Some((line, token)) => {
                            return error(line, format!("unexpected {token} in End Site"));
                        }
                        None => {
                            return error(
                                tokens.last_line,
                                "unbalanced braces: End Site is never closed",
                            );
                        }
                    }
                }
            }
            "}" => return Ok(()),
            other => return error(line, format!("unexpected {other} in joint {name}")),
        }
    }
}

fn parse_motion(
    lines: &[(usize, &str)],
    first_index: usize,
    channel_count: usize,
) -> Result<(f32, Vec<Vec<f32>>)> {
    let mut cursor = lines
        .iter()
        .copied()
        .filter(|(_, line)| !line.trim().is_empty());
    let last_line = first_index + lines.len();

    let Some((frames_line, line)) = cursor.next() else {
        return error(last_line, "missing Frames in MOTION");
    };
    let mut words = line.split_whitespace();
    let frame_count = match (words.next(), words.next()) {
        (Some("Frames:"), Some(count)) => count.parse::<usize>().ok(),
        (Some("Frames"), Some(":")) => words.next().and_then(|count| count.parse().ok()),
        _ => None,
    };
    let Some(frame_count) = frame_count else {
        return error(frames_line, "expected 'Frames: <count>'");
    };

    let Some((time_line, line)) = cursor.next() else {
        return error(last_line, "missing Frame Time in MOTION");
    };
    let words: Vec<&str> = line.split_whitespace().collect();
    let value = match words.as_slice() {
        ["Frame", "Time:", value] => Some(*value),
        ["Frame", "Time", ":", value] => Some(*value),
        _ => None,
    };
    let frame_time = value
        .and_then(|value| value.parse::<f32>().ok())
        .filter(|value| value.is_finite() && *value > 0.0);
    let Some(frame_time) = frame_time else {
        return error(
            time_line,
            "Frame Time must be a finite number greater than 0",
        );
    };

    let mut frames = Vec::new();
    for _ in 0..frame_count {
        let Some((line_number, line)) = cursor.next() else {
            return error(
                last_line,
                format!(
                    "Frames declares {frame_count} but only {} frame lines follow",
                    frames.len()
                ),
            );
        };
        let mut values = Vec::with_capacity(channel_count);
        for word in line.split_whitespace() {
            match word.parse::<f32>() {
                Ok(value) if value.is_finite() => values.push(value),
                _ => return error(line_number, format!("invalid frame value {word}")),
            }
        }
        if values.len() != channel_count {
            return error(
                line_number,
                format!(
                    "expected {channel_count} values on a frame line, found {}",
                    values.len()
                ),
            );
        }
        frames.push(values);
    }
    Ok((frame_time, frames))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOURCE: &str = "HIERARCHY
ROOT Hips
{
\tOFFSET 0.0 90.0 0.0
\tCHANNELS 6 Xposition Yposition Zposition Zrotation Yrotation Xrotation
\tJOINT Spine
\t{
\t\tOFFSET 0.0 10.0 0.0
\t\tCHANNELS 3 Xrotation Yrotation Zrotation
\t\tEnd Site
\t\t{
\t\t\tOFFSET 0.0 5.0 0.0
\t\t}
\t}
}
MOTION
Frames: 2
Frame Time: 0.5
0 90 0 1 2 3 4 5 6
7 8 9 10 11 12 13 14 15
";

    #[test]
    fn joints_are_stored_parents_first_with_channel_offsets() {
        let bvh = parse(SOURCE).unwrap();
        assert_eq!(bvh.joints.len(), 2);
        assert_eq!(bvh.joints[0].name, "Hips");
        assert_eq!(bvh.joints[0].parent, None);
        assert_eq!(bvh.joints[0].offset, [0.0, 90.0, 0.0]);
        assert_eq!(bvh.joints[0].channel_start, 0);
        assert_eq!(
            bvh.joints[0].channels,
            vec![
                Channel::XPosition,
                Channel::YPosition,
                Channel::ZPosition,
                Channel::ZRotation,
                Channel::YRotation,
                Channel::XRotation
            ]
        );
        assert_eq!(bvh.joints[1].name, "Spine");
        assert_eq!(bvh.joints[1].parent, Some(0));
        assert_eq!(bvh.joints[1].offset, [0.0, 10.0, 0.0]);
        assert_eq!(bvh.joints[1].channel_start, 6);
        assert_eq!(bvh.channel_count, 9);
    }

    #[test]
    fn motion_values_and_frame_time_are_read() {
        let bvh = parse(SOURCE).unwrap();
        assert_eq!(bvh.frame_time, 0.5);
        assert_eq!(bvh.frames.len(), 2);
        assert_eq!(bvh.frames[1][8], 15.0);
    }

    #[test]
    fn braces_without_surrounding_whitespace_are_tokens() {
        let compact = SOURCE.replace("ROOT Hips\n{", "ROOT Hips {");
        assert_eq!(parse(&compact).unwrap().joints.len(), 2);
    }

    #[test]
    fn end_site_adds_no_joint() {
        assert!(
            !parse(SOURCE)
                .unwrap()
                .joints
                .iter()
                .any(|j| j.name == "Site")
        );
    }
}
