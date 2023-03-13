#define BAUD_RATE 115200

const char* const HANDSHAKE_PROMPT   = "PC TO ARDUINO_1\n"; 
const char* const HANDSHAKE_RESPONSE = "ARDUINO_1 TO PC\n"; 
bool handshake_flag = false; 

void setup() {
  // put your setup code here, to run once:
  Serial.begin(BAUD_RATE); 
  digitalWrite(LED_BUILTIN, LOW); 
}

void loop() {
  // put your main code here, to run repeatedly:
  if (!handshake_flag) {
    handshake_flag = handshake(); 
    if (handshake_flag) Serial.println("Listening..."); 
  } else {
    listen(); 
  }
}

void listen() {
  if (Serial.available()) {
    String read = Serial.readStringUntil('\n');
    digitalWrite(LED_BUILTIN, HIGH); 
    delay(read.length()); 
    digitalWrite(LED_BUILTIN, LOW); 
    Serial.println(read.c_str()); 
  }
}

bool handshake() {
  if (Serial.available()) {
    String prompt = Serial.readString(); 
    if (prompt == HANDSHAKE_PROMPT) {
      Serial.write(HANDSHAKE_RESPONSE); 
    }
    while (true) {
      // Wait for handshake response
      if (Serial.available()) {
        String readback = Serial.readString(); 
        if (readback == HANDSHAKE_RESPONSE) {
          Serial.println("Handshake Successful!"); 
          return true; 
        }
      }
    }
  }
  return false; 
  // Enough with nesting ifs!
}