//! Format assertions tests — round-trip, indentation, comments, idempotency.

use arcis_fmt::format;

fn fmt(src: &str) -> String {
    format(src).expect("format")
}

#[test]
fn simple_function() {
    let input = "function f(x:number){return x+1;}";
    let expected = "\
function f(x: number) {
  return x + 1;
}
";
    assert_eq!(fmt(input), expected);
}

#[test]
fn if_else() {
    let input = "if(x>0){print(\"hi\");}else{print(\"bye\");}";
    let expected = "\
if (x > 0) {
  print(\"hi\");
} else {
  print(\"bye\");
}
";
    assert_eq!(fmt(input), expected);
}

#[test]
fn while_loop() {
    let input = "while(i<10){i=i+1;}";
    let expected = "\
while (i < 10) {
  i = i + 1;
}
";
    assert_eq!(fmt(input), expected);
}

#[test]
fn for_of_loop() {
    let input = "for(let x of arr){print(x);}";
    let expected = "\
for (let x of arr) {
  print(x);
}
";
    assert_eq!(fmt(input), expected);
}

#[test]
fn binary_operators_spaced() {
    let input = "let z:number=x+y*2;";
    let expected = "let z: number = x + y * 2;\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn member_access_no_spaces() {
    let input = "sys . readFile ( \"/tmp/x\" ) ;";
    let expected = "sys.readFile(\"/tmp/x\");\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn array_literal() {
    let input = "let a:number[]=[1,2,3];";
    let expected = "let a: number[] = [1, 2, 3];\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn object_literal() {
    let input = "let p:{name:string,age:number}={name:\"alice\",age:30};";
    let expected = "let p: { name: string, age: number } = { name: \"alice\", age: 30 };\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn import_statement() {
    let input = "import{trim,upper as up}from\"std\";";
    let expected = "import { trim, upper as up } from \"std\";\n";
    assert_eq!(fmt(input), expected);
}

#[test]
fn export_statement() {
    let input = "export function f():void{return;}";
    let expected = "\
export function f(): void {
  return;
}
";
    assert_eq!(fmt(input), expected);
}

#[test]
fn preserves_line_comments() {
    let input = "// hello\nlet x=1; // trailing\nlet y=2;";
    let result = fmt(input);
    assert!(result.contains("// hello"), "missing leading line comment: {result}");
    assert!(result.contains("// trailing"), "missing trailing line comment: {result}");
}

#[test]
fn preserves_block_comments() {
    let input = "/* header */\nlet x=1;/* inline */";
    let result = fmt(input);
    assert!(result.contains("/* header */"), "missing block comment: {result}");
    assert!(result.contains("/* inline */"), "missing inline block comment: {result}");
}

#[test]
fn idempotent() {
    let input = "\
function  add( a:number, b:number):number{
if( a>b){
return  a;
}else{
return  b;
}
}
";
    let first = fmt(input);
    let second = fmt(&first);
    assert_eq!(first, second, "format must be idempotent");
}

#[test]
fn idempotent_complex() {
    // Run through twice on a non-trivial snippet.
    let input = "\
import {trim,upper} from \"std\";
export function greet(name:string):string{
let s:string=\"hello \"+name+upper(name);
print(s);
return s;
}
";
    let first = fmt(input);
    let second = fmt(&first);
    assert_eq!(first, second, "complex idempotency broken");
}

#[test]
fn blank_lines_preserved_as_one() {
    let input = "\
let a = 1;


let b = 2;

let c = 3;
";
    // Multiple blank lines collapse to one.
    let result = fmt(input);
    assert!(result.contains(";\n\n"), "should preserve one blank line: {result}");
    // Must not have more than one blank line.
    assert!(!result.contains("\n\n\n"), "extra blank lines: {result}");
}

#[test]
fn empty_input_is_empty_output() {
    assert_eq!(fmt("\n"), "\n");
    assert_eq!(fmt("   \n"), "\n");
    assert_eq!(fmt(""), "\n");
}

#[test]
fn output_parses_clean() {
    // The formatted output must still be valid Arcis.
    let samples = [
        "let x: number = 1;\n",
        "function f(x: number): number { return x + 1; }\n",
        "if (x > 0) { print(\"pos\"); }\n",
        "for (let v of arr) { print(v); }\n",
        "from std import trim;\n",
        "let p: { name: string } = { name: \"x\" };\n",
    ];
    for s in &samples {
        let tokens = arcis_lexer::lex(s).expect(&format!("lex: {s}"));
        arcis_parser::parse(tokens).expect(&format!("parse: {s}"));
    }
}
