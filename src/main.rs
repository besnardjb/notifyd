use clap::Parser;
use tempdir::TempDir;
use std::{path::PathBuf, fs::remove_file};
use which::which;
use std::process::Command;
use std::path::Path;
use std::env;
use std::fs::File;
use std::error::Error;
use std::fmt::{self, format};
use md5::compute as md5;
use std::sync::Arc;
use rouille::{Response, Request};
use serde::{Serialize, Deserialize};
use soloud::*;
use gethostname::gethostname;
use std::time::SystemTime;

/*******************
 * HELPER FOR TIME *
 *******************/

fn now_in_usecs() -> u128 {
    match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
        Ok(n) => n.as_micros(),
        Err(_) => panic!("SystemTime before UNIX EPOCH!"),
    }
}


/****************
 * DEFINE ERROR *
 ****************/

#[derive(Debug)]
struct NotifydError(String);

impl fmt::Display for NotifydError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Error for NotifydError {}

impl NotifydError {
    fn new(desc : &str) -> Box<dyn Error>
    {
        Box::new(NotifydError(String::from(desc)))
    }
}

/**************
 * TTS ENGINE *
 **************/

#[derive(Debug,PartialEq)]
enum TTSEngine
{
    PICO2WAV,
    ESPEAK,
    ESPEAKNG,
    AUTO
}

struct TtsSentence
{
    text: String,
    path : String
}

impl TtsSentence
{
    fn new(path : &str, text : &str) -> TtsSentence
    {
        TtsSentence{
            text : String::from(text),
            path : String::from(path)
        }
    }

    fn _run_player(self : &Self, player : &str) -> Result<(), Box<dyn std::error::Error>>
    {
        let cmd = [player, self.path.as_str()];

        let ret = Command::new(cmd[0])
        .args(&cmd[1..])
        .output()?;

        if !ret.status.success()
        {
            println!("Failed to run {:?}", cmd);
            println!("{}", String::from_utf8(ret.stderr).unwrap());
            return Err(NotifydError::new("Failed to run tts"));
        }
        Ok(())
    }

    fn _play_external(self: & Self) -> Result<(), Box<dyn std::error::Error>>
    {
        let candidate_players = ["paplay", "mplayer", "play" /* sox */];

        for p in candidate_players
        {
            match which(p) {
                Ok(p) => {
                    match self._run_player(p.to_str().unwrap())
                    {
                        Ok(_) => return Ok(()),
                        Err(e) => return Err(e)
                    }
                },
                Err(_) => {}
            }
        }

        Err(NotifydError::new(format!("Could not find any player in {:?} to play {}", candidate_players, self.path).as_str()))
    }

    fn play(self : &Self, sl : & Soloud) -> Result<(), Box<dyn std::error::Error>>
    {
        //self.play_external()
        let mut wav = audio::Wav::default();
        wav.load(&std::path::Path::new(&self.path))?;

        sl.play(&wav);
        while sl.voice_count() > 0 {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        Ok(())

    }

    fn delete(self : &Self) -> Result<(), Box<dyn std::error::Error>>
    {
        println!("Removing data for {} : '{}'", self.path, self.text);
        remove_file(&self.path)?;
        Ok(())
    }
}


struct TTS
{
    enginepath : String,
    lang : String,
    tmpdir : TempDir
}

impl TTS
{
    fn tts_to_bin_name( engine : & TTSEngine) -> &'static str
    {
        match engine {
            TTSEngine::PICO2WAV => "pico2wave",
            TTSEngine::ESPEAK => "espeak",
            TTSEngine::ESPEAKNG =>  "espeak-ng",
            TTSEngine::AUTO => panic!("AUTO engine cannot be instanciated")
        }
    }

    fn look_for_candidate_engine(engine : TTSEngine) -> Result<TTSEngine, Box<dyn std::error::Error>>
    {
        if engine != TTSEngine::AUTO
        {
            return Ok(engine)
        }

        let engines = vec![TTSEngine::PICO2WAV, TTSEngine::ESPEAK, TTSEngine::ESPEAKNG];

        for e in engines{
            match which(TTS::tts_to_bin_name(&e))
            {
                Ok(_) => return Ok(e),
                Err(_) => {break;},
            }
        }

        panic!("Cannot find any binary for implementing TTS in PATH");
    }

    fn speak_to_file(self :& Self, text : String) -> Result<TtsSentence, Box<dyn std::error::Error>>
    {
        let to_hash = format!("{}{}", text, now_in_usecs());
        let digest = md5(to_hash);
        let outfile = self.tmpdir.path().join(format!("{}.wav", format!("{:x}", digest)));
        let outpath: &str = outfile.to_str().expect("Failed to convert path to str");

        let cmd: [&str; 6] = [self.enginepath.as_str(), "-w", outpath, "-l", self.lang.as_str(), text.as_str()];

        let ret = Command::new(cmd[0])
        .args(&cmd[1..])
        .output()?;

        if !ret.status.success()
        {
            let err_desc = format!("{}", String::from_utf8(ret.stderr).unwrap());
            println!("{:?}", cmd);
            println!("~~~ Failed to run TSS engine ~~~");
            println!("{}", err_desc);
            println!("~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~");
            return Err(NotifydError::new(err_desc.as_str()));
        }

        Ok(TtsSentence::new(outpath, text.as_str()))
    }

    fn get_locale_from_env() -> String
    {
        match env::var("LANG")
        {
            Ok(l) => {
                if l.find(".").is_some()
                {
                    match l.split(".").next()
                    {
                        Some(e) => return e.to_string().replace("_", "-"),
                        None => panic!("Failed to split on '.'")
                    }
                }
                return l.replace("_", "-")
            },
            Err(_) => return String::from("en-US")
        }
    }

    fn new(engine : TTSEngine) -> Result<TTS, Box<dyn std::error::Error>>
    {
        let tmp_dir: TempDir = TempDir::new("notifydtts")?;

        let engine_to_use = TTS::look_for_candidate_engine(engine)?;

        let engine_binary_name = String::from(TTS::tts_to_bin_name(&engine_to_use));

        let enginepath : PathBuf;

        match which(&engine_binary_name)
        {
            Ok(path) => enginepath = path,
            Err(_) => panic!("Cannot find TTS engine {} in PATH", engine_binary_name)
        }

        let locale = TTS::get_locale_from_env();

        println!("Using TTS engine {}", engine_binary_name);

        return Ok(TTS { tmpdir: tmp_dir,
                        lang : locale,
                        enginepath: String::from(enginepath.to_string_lossy())
                     })
    }

}

/********************
 * WAV FILE CASTING *
 ********************/

struct Caster
{
    target_uid : String,
    url : String,
}

impl Drop for Caster
{
    fn drop(&mut self)
    {
        let _ = self.stop();
    }
}


impl Caster
{

    fn has_go_chromecast() -> Result<(), Box<dyn std::error::Error>>
    {
        match which("go-chromecast")
        {
            Ok(_) => Ok(()),
            Err(_) => Err(NotifydError::new("Cannot locate go-chromecast in path"))

        }
    }

    fn do_run(self : & Self, args : Vec<&str>)  ->  Result<(), Box<dyn std::error::Error>>
    {
        let cmd: [&str; 4] = ["go-chromecast", "-u", self.target_uid.as_str(), self.url.as_str()];

        let ret = Command::new("go-chromecast")
        .args(&args)
        .output()?;

        if !ret.status.success()
        {
            let err_desc = format!("{}", String::from_utf8(ret.stderr).unwrap());
            println!("{:?} {:?}", cmd, args);
            println!("~~~ Failed to run go-chromecast ~~~");
            println!("{}", err_desc);
            println!("~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~");
            return Err(NotifydError::new(err_desc.as_str()));
        }

        Ok(())
    }

    fn load(self : &Self) ->  Result<(), Box<dyn std::error::Error>>
    {
        self.do_run(vec!["load", "-u", self.target_uid.as_str(), self.url.as_str()])?;
        Ok(())
    }

    fn stop(self : &Self) ->  Result<(), Box<dyn std::error::Error>>
    {
        self.do_run(vec!["stop", "-u", self.target_uid.as_str()])?;
        Ok(())
    }

    fn new(uid:String, url : String) ->  Result<Caster, Box<dyn std::error::Error>>
    {
        Caster::has_go_chromecast()?;
        Ok(Caster{
            target_uid : uid,
            url : url
        })
    }

}



/**********************************
 * DEFINE THE NOTIFICATION DAEMON *
 **********************************/

struct Notifyd
{
    port : u32,
    target_uuid : String,
    tts : TTS,
    sound : Soloud
}
#[derive(Serialize)]
struct ProtoResponse
{
    success: bool,
    reason : String,
    err : String
}

impl Notifyd
{
    fn new( port : u32, target_uuid : String) ->  Result<Notifyd, Box<dyn std::error::Error>>
    {
        let sl = Soloud::default()?;

        Ok(
            Notifyd{
                port : port,
                tts : TTS::new(TTSEngine::AUTO)?,
                target_uuid : target_uuid,
                sound: sl
            }
        )
    }

    fn error_response(reason : &str, err : Box<dyn std::error::Error>) -> Response
    {
        Response::json(&ProtoResponse{
            success : false,
            reason : reason.to_string(),
            err : err.to_string()
        }).with_status_code(400)
    }

    fn success_response(reason : &str) -> Response
    {
        Response::json(&ProtoResponse{
            success : true,
            reason : reason.to_string(),
            err : "".to_string()
        })
    }

    fn do_tts(self : & Self, text : String)  -> Response
    {
        let sentence: Result<TtsSentence, Box<dyn Error>> = self.tts.speak_to_file(text);

        match sentence {
            Ok(a) => {
                match a.play(&self.sound)
                {
                    Ok(()) => {
                        return Notifyd::success_response("Done emitting requested text");
                    },
                    Err(e) => {
                        return Notifyd::error_response("Failed playing text", e);
                    }
                }
            },
            Err(err) => {
                Notifyd::error_response("Failed to generate TTS from text", err)
            }
        }
    }

    fn handle_tts_request(self : & Self, request : &Request) -> Response
    {
        #[derive(Deserialize)]
        struct Json {
            text: String,
        }

        let json : Json;
        match rouille::input::json_input(request)
        {
            Ok(a) => {
                json = a;
            }
            Err(e) =>{
                return Notifyd::error_response("Bad arguments", Box::new(e));
            }
        }

        self.do_tts(json.text)
    }

    fn handle_static_req(self : & Self, request : &Request) -> Response
    {
        let raw_url = request.url();

        if !raw_url.starts_with("/static/")
        {
            panic!("handle_static_req to be called only on static requests");
        }

        let target_path: PathBuf = self.tts.tmpdir.path().join(&raw_url["/static/".len()..]);

        if !target_path.is_file()
        {
            return Response::empty_404();
        }

        match File::open(&target_path){
            Ok(f) => {
                Response::from_file("audio/wav", f)
            }
            Err(e) => {
                Notifyd::error_response(format!("Sending static file {}",
                                                  target_path.as_path().to_string_lossy()).as_str(),
                                   Box::new(e))
            }
        }
    }

    fn sentence_static_url(self : & Self, sentence : TtsSentence) -> String
    {
        use local_ip_address::local_ip;
        let fpath;
        match  Path::new(&sentence.path).file_name() {
            Some(p) => {
                fpath = String::from(p.to_string_lossy());
            }
            None => {
                fpath = String::from("");
            }
        }

        let my_local_ip = local_ip().unwrap();
        format!("http://{}:{}/static/{}", my_local_ip, self.port, fpath)
    }

    fn do_bcast(self : & Self, text : String, uid : String) -> Response
    {
        let sentence : TtsSentence;

        match self.tts.speak_to_file(text) {
            Ok(s) => {
                sentence = s;
            },
            Err(e) => {
                return Notifyd::error_response("Failed to generate TTS", e);
            }
        }

        let url = self.sentence_static_url(sentence);

        match Caster::new(uid, url) {
            Ok(c) => {
                match c.load() {
                    Ok(()) => {
                        return Notifyd::success_response("Content casted");
                    }
                    Err(e) => {
                        return Notifyd::error_response("Failed to cast content", e);
                    }
                }
            },
            Err(e) => {
                return Notifyd::error_response("Failed start cast", e);
            }
        }
    }

    fn handle_bcast_req(self : & Self, request : &Request) -> Response
    {
        #[derive(Deserialize)]
        struct Json {
            text: String,
            uid : String
        }

        let json : Json;
        match rouille::input::json_input(request)
        {
            Ok(a) => {
                json = a;
            }
            Err(e) =>{
                return Notifyd::error_response("Bad arguments", Box::new(e));
            }
        }

        self.do_bcast(json.text, json.uid)
    }

    fn handle_notify_req(self : &Self, request : &Request)  -> Response
    {
        #[derive(Deserialize)]
        struct Json {
            text: String,
        }

        let json : Json;
        match rouille::input::json_input(request)
        {
            Ok(a) => {
                json = a;
            }
            Err(e) =>{
                return Notifyd::error_response("Bad arguments", Box::new(e));
            }
        }

        if self.target_uuid == "Use Local Speaker"
        {
            self.do_tts(json.text)
        }
        else
        {
            self.do_bcast(json.text, self.target_uuid.to_string())
        }
    }

    fn route_request(self : &Self, request : &Request) -> Response
    {
        let url = request.url();
        //println!("Request to {}", url);
        match url.as_str()
        {
            "/action/speak" => {
                self.handle_tts_request(request)
            },
            "/action/cast" => {
                self.handle_bcast_req(request)
            },
            "/notify" => {
                self.handle_notify_req(request)
            }
            v => {
                // The case of static files
                if v.starts_with("/static/")
                {
                    return self.handle_static_req(request)
                }

                return Notifyd::error_response("No such endpoint",
                                     NotifydError::new(format!("No endpoint {}", v).as_str()));
            }
        }

    }

    fn run(self : Arc<Self>)
    {
        let me = Arc::clone(&self);
        rouille::start_server(format!("0.0.0.0:{}",me.port), move |request| {
            me.route_request(request)
        });
    }
}

/*****************
 * CLI ARGUMENTS *
 *****************/

 #[derive(Parser,Debug)]
 struct Cli {
     /// The UID of the chromecast to target
     #[clap(short, long, default_value = "Use Local Speaker")]
     chromecast_uuid : String,
     /// The port of the webserver
     #[clap(short, long, default_value_t = 8090)]
     port : u32,
 }

/*******************
 * DEFINE THE MAIN *
 *******************/

fn main() -> Result<(), Box<dyn std::error::Error>> {

    let args = Cli::parse();

    let server = Notifyd::new(args.port, args.chromecast_uuid)?;

    Notifyd::run(Arc::new(server));

    Ok(())
}
