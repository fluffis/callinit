use eframe::egui;
use std::sync::mpsc;
use std::thread;
use arboard::Clipboard;

#[macro_use]
extern crate ini;
extern crate dirs;

fn main() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([400.0, 200.0])
            .with_title("Call initializer"),
        ..Default::default()
    };

    eframe::run_native(
        "Call initializer",
        options,
        Box::new(|_cc| Box::new(MyApp::new())),
    )
}

struct MyApp {
    input_text: String,
    should_focus: bool,
    http_sender: Option<mpsc::Sender<String>>,
    http_receiver: mpsc::Receiver<String>,
    waiting_for_response: bool,
    auth_token: Option<String>,
    country_code: Option<String>,
    notify_topic: Option<String>,
}

impl MyApp {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let (auth_token, country_code, notify_topic) = Self::read_config();
        let input_text = Self::check_clipboard_for_phone_number();

        Self {
            input_text,
            should_focus: true,
            http_sender: Some(tx),
            http_receiver: rx,
            waiting_for_response: false,
            auth_token,
            country_code,
            notify_topic,
        }
    }

    fn check_clipboard_for_phone_number() -> String {
        let Ok(mut clipboard) = Clipboard::new() else {
            return String::new();
        };
        let Ok(text) = clipboard.get_text() else {
            return String::new();
        };
        if Self::looks_like_phone_number(&text) {
            text.trim().to_string()
        } else {
            String::new()
        }
    }

    fn looks_like_phone_number(text: &str) -> bool {
        let trimmed = text.trim();
        if trimmed.is_empty() || trimmed.len() > 20 {
            return false;
        }
        let digit_count = trimmed.chars().filter(|c| c.is_ascii_digit()).count();
        let valid_chars = trimmed.chars().all(|c| {
            c.is_ascii_digit() || c == '+' || c == '-' || c == ' ' || c == '(' || c == ')'
        });
        digit_count >= 6 && valid_chars
    }

    fn read_config() -> (Option<String>, Option<String>, Option<String>) {
        let filename = dirs::home_dir().unwrap().to_str().unwrap().to_owned() + "/.config/callinit.ini";
        let map = ini!(&filename);
        let auth_token = map["auth"]["token"].clone();
        let country_code = map["phone"]["country_code"].clone();
        let notify_topic = map["notify"]["topic"].clone();
        (auth_token, country_code, notify_topic)
    }

    fn format_e164(&self, number: &str) -> String {
        let digits: String = number.chars().filter(|c| c.is_ascii_digit() || *c == '+').collect();
        if digits.starts_with('+') {
            digits
        } else if let Some(ref cc) = self.country_code {
            if digits.starts_with('0') {
                format!("+{}{}", cc, &digits[1..])
            } else {
                format!("+{}{}", cc, digits)
            }
        } else {
            digits
        }
    }

    fn send_http_request(&mut self) {
        if let Some(sender) = self.http_sender.take() {
            let text = self.format_e164(&self.input_text);
            if text.is_empty() {
                return;
            }

            let auth_token = self.auth_token.clone();
            let notify_topic = self.notify_topic.clone();

            thread::spawn(move || {
                let client = reqwest::blocking::Client::new();
                let mut builder = client
                    .post("https://ntfy.sh")
                    .header("Title", format!("Call {}", text));

                if let Some(token) = auth_token {
                    builder = builder.header("Authorization", format!("Bearer {}", token));
                    println!("Adding Authorization header with token");
                } else {
                    println!("No auth token available, sending request without authentication");
                }
		let result = builder
                    .json(&serde_json::json!({
                        "topic": notify_topic.unwrap_or_default(),
                        "message": text,
                        "actions": [
                           {
                              "action": "view",
                              "label": "Call",
                              "url": format!("tel:{}", text),
                              "clear": true
                           },
                           {
                              "action": "view",
                              "label": "SMS",
                              "url": format!("sms:{}", text),
                              "clear": true
                           },
                           {
                              "action": "view",
                              "label": "WhatsApp",
                              "url": format!("https://wa.me/{}", text.trim_start_matches('+')),
                              "clear": true
                           }
                        ]
                     }))
                    .send();

                match result {
                    Ok(response) => {
                        println!("HTTP Response Status: {}", response.status());
                        if let Ok(body) = response.text() {
                            println!("Response Body: {}", body);
                        }
                    }
                    Err(e) => {
                        eprintln!("HTTP Request failed: {}", e);
                    }
                }

                // Signal that the request is complete
                let _ = sender.send("complete".to_string());
            });
            
            self.waiting_for_response = true;
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Check if HTTP request completed
        if let Ok(_) = self.http_receiver.try_recv() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(50.0);
                
                ui.label("Enter number to send and press Enter:");
                ui.add_space(10.0);

                let response = ui.add_sized(
                    [300.0, 30.0],
                    egui::TextEdit::singleline(&mut self.input_text)
                        .hint_text("Type your number here...")
                );

                // Focus the input box on first frame
                if self.should_focus {
                    response.request_focus();
                    self.should_focus = false;
                }

                // Handle Enter key press
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if !self.input_text.is_empty() && !self.waiting_for_response {
                        println!("Sending HTTP request with text: {}", self.input_text);
                        self.send_http_request();
                    }
                }
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }

                if self.waiting_for_response {
                    ui.add_space(20.0);
                    ui.label("Sending HTTP request...");
                }
            });
        });
    }
}
