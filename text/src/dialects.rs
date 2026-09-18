//! Per-language data for the shared tokeniser in `code.rs`. One chained entry per dialect.

pub const NEST: u32 = 1 << 0; // block comments nest
pub const TRIPLE: u32 = 1 << 1; // """ and ''' strings
pub const CHAR: u32 = 1 << 2; // ' is a character literal, not a string
pub const ICASE: u32 = 1 << 3;
pub const PREPROC: u32 = 1 << 4; // # directive lines
pub const ATTR: u32 = 1 << 5; // #[...] attributes
pub const DECOR: u32 = 1 << 6; // @name annotations
pub const MARKUP: u32 = 1 << 7; // <tag attr="value">
pub const EMBED: u32 = 1 << 8; // <?php ... ?> and <% ... %> delimiters
pub const KEYS: u32 = 1 << 9; // name: or name = at the head of a line, "name": anywhere
pub const TABLE: u32 = 1 << 10; // [section] lines
pub const MD: u32 = 1 << 11;
pub const RST: u32 = 1 << 12;
pub const TEXTILE: u32 = 1 << 13;
pub const DIFF: u32 = 1 << 14;
pub const TEX: u32 = 1 << 15; // \command
pub const REGEX: u32 = 1 << 16;
pub const PREFIX: u32 = 1 << 17; // the head of a parenthesised form is the callee

pub struct Dialect {
    pub name: &'static str,
    pub aliases: &'static str,
    pub keywords: &'static str,
    pub types: &'static str,
    pub constants: &'static str,
    pub line: &'static [&'static str],
    pub block: &'static [(&'static str, &'static str)],
    pub quotes: &'static str,
    pub raw: &'static str, // prefix characters accepted before a quote
    pub sigils: &'static str,
    pub word: &'static str, // extra characters an identifier may contain (`if-let`, `set!`, `font-face`)
    pub flags: u32,
}

impl Dialect {
    const fn new(name: &'static str, aliases: &'static str) -> Self {
        Dialect {
            name,
            aliases,
            keywords: "",
            types: "",
            constants: "",
            line: &[],
            block: &[],
            quotes: "",
            raw: "",
            sigils: "",
            word: "",
            flags: 0,
        }
    }
    const fn words(mut self, keywords: &'static str, types: &'static str, constants: &'static str) -> Self {
        self.keywords = keywords;
        self.types = types;
        self.constants = constants;
        self
    }
    const fn line(mut self, line: &'static [&'static str]) -> Self {
        self.line = line;
        self
    }
    const fn block(mut self, block: &'static [(&'static str, &'static str)]) -> Self {
        self.block = block;
        self
    }
    const fn strings(mut self, quotes: &'static str, raw: &'static str) -> Self {
        self.quotes = quotes;
        self.raw = raw;
        self
    }
    const fn sigils(mut self, sigils: &'static str) -> Self {
        self.sigils = sigils;
        self
    }
    const fn word(mut self, word: &'static str) -> Self {
        self.word = word;
        self
    }
    const fn flags(mut self, flags: u32) -> Self {
        self.flags = flags;
        self
    }
    pub fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }
    pub fn role(&self, word: &str) -> Option<&'static str> {
        let icase = self.has(ICASE);
        if member(self.constants, word, icase) {
            Some("constant")
        } else if member(self.keywords, word, icase) {
            Some("keyword")
        } else if member(self.types, word, icase) {
            Some("type")
        } else {
            None
        }
    }
    pub fn known(&self, word: &str) -> bool {
        self.role(word).is_some()
    }
}

fn member(list: &str, word: &str, icase: bool) -> bool {
    list.split_ascii_whitespace().any(|item| {
        if icase {
            item.eq_ignore_ascii_case(word)
        } else {
            item == word
        }
    })
}

pub fn find(name: &str) -> Option<&'static Dialect> {
    DIALECTS
        .iter()
        .find(|d| member(d.aliases, name.trim(), true))
}

const SLASH: &[&str] = &["//"];
const HASH: &[&str] = &["#"];
const DASH: &[&str] = &["--"];
const SEMI: &[&str] = &[";"];
const CBLOCK: &[(&str, &str)] = &[("/*", "*/")];
const SGML: &[(&str, &str)] = &[("<!--", "-->")];
const PAREN: &[(&str, &str)] = &[("(*", "*)")];

/// Dialect order fixes the detection bit index; keep it under 64 entries.
pub static DIALECTS: &[Dialect] = &[
    Dialect::new("rust", "rust rs")
        .words(
            "as async await break const continue crate dyn else enum extern fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait type unsafe use where while",
            "bool char str u8 u16 u32 u64 u128 usize i8 i16 i32 i64 i128 isize f32 f64 String Vec Option Result Box",
            "true false None Some Ok Err",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "br#")
        .flags(NEST | CHAR | ATTR),
    Dialect::new("kotlin", "kotlin kt kts")
        .words(
            "as break by catch class companion constructor continue crossinline data delegate do dynamic else enum expect actual external field file final finally for fun get if import in infix init inline inner interface internal is lateinit noinline object open operator out override package param private property protected public receiver reified return sealed set setparam super suspend tailrec this throw try typealias typeof val var vararg when where while abstract annotation const",
            "Int Long Short Byte Double Float Boolean String Char Any Unit Nothing List Map Set Array MutableList",
            "true false null",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "")
        .flags(NEST | CHAR | TRIPLE | DECOR),
    Dialect::new("c", "c h")
        .words(
            "auto break case const continue default do else enum extern for goto if inline register restrict return sizeof static struct switch typedef union volatile while _Atomic _Bool _Generic",
            "char double float int long short signed unsigned void size_t ssize_t ptrdiff_t bool wchar_t int8_t int16_t int32_t int64_t uint8_t uint16_t uint32_t uint64_t FILE va_list",
            "true false NULL EOF",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "Lu")
        .flags(CHAR | PREPROC),
    Dialect::new("cpp", "cpp c++ cxx cc hpp hh hxx")
        .words(
            "alignas alignof asm auto break case catch class concept const consteval constexpr constinit continue co_await co_return co_yield decltype default delete do else enum explicit export extern friend goto if inline mutable namespace new noexcept operator private protected public register reinterpret_cast requires return sizeof static static_assert static_cast const_cast dynamic_cast struct switch template this thread_local throw try typedef typeid typename union using virtual volatile while",
            "bool char char8_t char16_t char32_t double float int long short signed unsigned void wchar_t size_t string wstring vector map unordered_map set array pair shared_ptr unique_ptr",
            "true false nullptr NULL",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "LuRr")
        .flags(CHAR | PREPROC),
    Dialect::new("csharp", "csharp cs c-sharp")
        .words(
            "abstract as base break case catch checked class const continue default delegate do else enum event explicit extern finally fixed for foreach goto if implicit in interface internal is lock namespace new operator out override params private protected public readonly record ref return sealed sizeof stackalloc static struct switch this throw try typeof unchecked unsafe using virtual volatile while async await var yield get set nameof partial where select from",
            "bool byte char decimal double float int long object sbyte short string uint ulong ushort void Task List Dictionary IEnumerable Span",
            "true false null value",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "@$")
        .flags(CHAR | PREPROC),
    Dialect::new("java", "java")
        .words(
            "abstract assert break case catch class const continue default do else enum extends final finally for goto if implements import instanceof interface native new package permits private protected public record return sealed static strictfp super switch synchronized this throw throws transient try var void volatile while yield",
            "boolean byte char double float int long short String Object Integer Double Boolean Long List Map ArrayList HashMap",
            "true false null",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "")
        .flags(CHAR | TRIPLE | DECOR),
    Dialect::new("javascript", "javascript js jsx mjs cjs node ecmascript")
        .words(
            "async await break case catch class const continue debugger default delete do else export extends finally for from function get if import in instanceof let new of return set static super switch this throw try typeof var void while with yield",
            "Array Object String Number Boolean Promise Map Set Symbol RegExp Date JSON Math Error",
            "true false null undefined NaN Infinity",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'`", ""),
    Dialect::new("typescript", "typescript ts tsx mts")
        .words(
            "abstract as asserts async await break case catch class const continue declare default delete do else enum export extends finally for from function get if implements import in infer instanceof interface is keyof let namespace new of private protected public readonly return satisfies set static super switch this throw try type typeof var void while yield",
            "any bigint boolean never number object string symbol unknown void Array Promise Record Partial Readonly Map Set",
            "true false null undefined",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'`", "")
        .flags(DECOR),
    Dialect::new("python", "python py python3 py3 pyi")
        .words(
            "and as assert async await break case class continue def del elif else except finally for from global if import in is lambda match nonlocal not or pass raise return try while with yield",
            "bool bytes complex dict float frozenset int list object set str tuple type",
            "True False None NotImplemented Ellipsis self cls",
        )
        .line(HASH)
        .strings("\"'", "rbfuRBFU")
        .flags(TRIPLE | DECOR),
    Dialect::new("ruby", "ruby rb gemfile rake")
        .words(
            "alias and begin break case class def defined do each else elsif end ensure for if in module next not or redo rescue retry return self super then undef unless until when while yield require require_relative attr_accessor attr_reader attr_writer lambda proc puts raise",
            "String Integer Float Array Hash Symbol Range Struct Class Module",
            "true false nil __FILE__ __dir__",
        )
        .line(HASH)
        .block(&[("=begin", "=end")])
        .strings("\"'", "")
        .sigils("@$"),
    Dialect::new("erb", "erb rhtml eruby")
        .words(
            "and begin case class def do else elsif end ensure for if in module next not or rescue return then unless until when while yield each render link_to form_for content_tag",
            "String Integer Float Array Hash Symbol",
            "true false nil",
        )
        .block(SGML)
        .strings("\"'", "")
        .sigils("@")
        .flags(MARKUP | EMBED),
    Dialect::new("php", "php php5 php7 php8 phtml")
        .words(
            "abstract and array as break callable case catch class clone const continue declare default do echo else elseif empty enddeclare endfor endforeach endif endswitch endwhile enum extends final finally fn for foreach function global goto if implements include include_once instanceof insteadof interface isset list match namespace new or print private protected public readonly require require_once return static switch throw trait try unset use var while xor yield",
            "int float string bool array object callable iterable void mixed never self parent",
            "true false null TRUE FALSE NULL",
        )
        .line(&["//", "#"])
        .block(CBLOCK)
        .strings("\"'", "")
        .sigils("$")
        .flags(MARKUP | EMBED),
    Dialect::new("go", "go golang")
        .words(
            "break case chan const continue default defer else fallthrough for func go goto if import interface map package range return select struct switch type var",
            "bool byte complex64 complex128 error float32 float64 int int8 int16 int32 int64 rune string uint uint8 uint16 uint32 uint64 uintptr any make new len cap append",
            "true false nil iota",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'`", "")
        .flags(CHAR),
    Dialect::new("swift", "swift")
        .words(
            "actor as associatedtype async await break case catch class continue convenience default defer deinit do else enum extension fallthrough fileprivate final for func guard if import in indirect init inout internal is lazy let mutating nonmutating open operator override private protocol public repeat required rethrows return some static struct subscript switch throw throws try typealias var where while",
            "Int Int8 Int32 Int64 UInt Double Float String Bool Character Array Dictionary Set Optional Void Data",
            "true false nil self Self super",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"", "")
        .flags(NEST | TRIPLE | DECOR),
    Dialect::new("dart", "dart flutter")
        .words(
            "abstract as assert async await break case catch class const continue covariant default deferred do dynamic else enum export extends extension external factory final finally for get hide if implements import in interface is late library mixin new on operator part required rethrow return set show static super switch sync this throw try typedef var while with yield",
            "int double num String bool List Map Set Future Stream Object Iterable void Widget",
            "true false null",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "r")
        .flags(NEST | TRIPLE | DECOR),
    Dialect::new("scala", "scala sc sbt")
        .words(
            "abstract case catch class def do else enum export extends final finally for forSome given if implicit import lazy match new object override package private protected return sealed super this throw trait try type using val var while with yield",
            "Int Long Double Float Boolean String Char Unit Any AnyRef AnyVal Nothing List Map Option Seq Vector Future",
            "true false null None Some",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "")
        .flags(NEST | TRIPLE | CHAR | DECOR),
    Dialect::new("groovy", "groovy gradle gvy")
        .words(
            "as assert break case catch class const continue def default do else enum extends final finally for goto if implements import in instanceof interface new package private protected public return static strictfp super switch synchronized this throw throws trait try void while it task apply",
            "boolean byte char double float int long short String Object List Map Closure BigDecimal",
            "true false null",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "")
        .sigils("$")
        .flags(TRIPLE | DECOR),
    Dialect::new("haskell", "haskell hs lhs")
        .words(
            "case class data default deriving do else foreign forall if import in infix infixl infixr instance let module newtype of then type where",
            "Int Integer Float Double Char String Bool Maybe Either IO Ordering Word",
            "True False Nothing Just otherwise",
        )
        .line(DASH)
        .block(&[("{-", "-}")])
        .strings("\"'", "")
        .flags(NEST | CHAR),
    Dialect::new("ocaml", "ocaml ml mli caml")
        .words(
            "and as assert begin class constraint do done downto else end exception external for fun function functor if in include inherit initializer lazy let match method module mutable new nonrec object of open private rec sig struct then to try type val virtual when while with",
            "int float char string bool unit list array option ref bytes exn",
            "true false None Some",
        )
        .block(PAREN)
        .strings("\"'", "")
        .flags(NEST | CHAR),
    Dialect::new("lisp", "lisp cl common-lisp elisp emacs-lisp scheme racket")
        .words(
            "defun defvar defparameter defmacro defconst defconstant defstruct defclass defmethod let let* lambda if cond case when unless progn setq setf quote function do dolist dotimes loop return car cdr cons list append mapcar apply funcall define set! begin display newline",
            "",
            "t nil true false",
        )
        .line(SEMI)
        .block(&[("#|", "|#")])
        .strings("\"", "")
        .word("-*!?")
        .flags(PREFIX),
    Dialect::new("clojure", "clojure clj cljs cljc edn")
        .words(
            "def defn defn- defmacro defprotocol defrecord deftype defmulti defmethod fn let letfn if if-let if-not when when-let when-not cond condp case do loop recur try catch finally throw ns require import in-ns quote var set! new binding doseq dotimes for map filter reduce apply reify",
            "",
            "true false nil",
        )
        .line(SEMI)
        .strings("\"", "")
        .word("-*!?")
        .flags(PREFIX),
    Dialect::new("lua", "lua luau")
        .words(
            "and break do else elseif end for function goto if in local not or repeat return then until while",
            "string table number boolean function coroutine io os math",
            "true false nil self",
        )
        .line(DASH)
        .block(&[("--[[", "]]")])
        .strings("\"'", ""),
    Dialect::new("perl", "perl pl pm perl5")
        .words(
            "my our local sub if elsif else unless while until for foreach do return last next redo goto package use require no bless ref defined undef exists delete wantarray print printf sprintf push pop shift unshift split join keys values each eval die warn open close chomp qw qq",
            "",
            "undef __END__ __DATA__ STDIN STDOUT STDERR",
        )
        .line(HASH)
        .strings("\"'`", "")
        .sigils("$@%"),
    Dialect::new("r", "r rscript rlang")
        .words(
            "if else repeat while function for in next break return library require source suppressWarnings invisible",
            "numeric character logical integer double complex list vector matrix factor",
            "TRUE FALSE NULL NA NaN Inf",
        )
        .line(HASH)
        .strings("\"'", ""),
    Dialect::new("matlab", "matlab octave")
        .words(
            "function end if elseif else switch case otherwise for parfor while break continue return try catch global persistent classdef properties methods events arguments spmd",
            "double single int8 int16 int32 int64 uint8 uint16 logical char cell struct string categorical",
            "true false pi Inf NaN eps nargin nargout",
        )
        .line(&["%", "#"])
        .block(&[("%{", "%}")])
        .strings("'\"", ""),
    Dialect::new("sql", "sql mysql postgres postgresql pgsql sqlite plsql tsql")
        .words(
            "select from where group by having order limit offset insert into values update set delete create alter drop truncate table view index trigger procedure function returns begin end declare as join inner left right full outer cross on using union all distinct case when then else exists in like ilike between is not and or asc desc primary key foreign references unique default constraint check with recursive grant revoke commit rollback transaction explain analyze",
            "int integer bigint smallint decimal numeric float real double char varchar text date time timestamp interval boolean blob bytea serial uuid json jsonb array",
            "true false null current_date current_timestamp",
        )
        .line(&["--", "#"])
        .block(CBLOCK)
        .strings("'\"", "")
        .flags(ICASE),
    Dialect::new("shell", "shell sh bash zsh ksh console shellsession bashrc")
        .words(
            "if then else elif fi for while until do done case esac function in select return break continue exit export local readonly declare typeset source alias unalias set unset shift trap eval exec getopts echo printf cd read pushd popd",
            "",
            "true false",
        )
        .line(HASH)
        .strings("\"'`", "")
        .sigils("$"),
    Dialect::new("powershell", "powershell ps1 pwsh posh psm1")
        .words(
            "begin break catch class continue data define do dynamicparam else elseif end enum exit filter finally for foreach from function hidden if in param process return static switch throw trap try until using while workflow",
            "int string bool array hashtable pscustomobject void double decimal char long switch",
            "true false null",
        )
        .line(HASH)
        .block(&[("<#", "#>")])
        .strings("\"'", "")
        .sigils("$")
        .flags(ICASE),
    Dialect::new("batch", "batch bat cmd dosbatch winbatch")
        .words(
            "echo off on set setlocal endlocal enabledelayedexpansion if else for in do goto call exit rem pause shift start title cls copy move del mkdir rmdir type errorlevel exist not defined equ neq lss leq gtr geq",
            "",
            "true false nul",
        )
        .line(&["rem", "::"])
        .strings("\"", "")
        .sigils("%")
        .flags(ICASE),
    Dialect::new("tcl", "tcl tk itcl")
        .words(
            "after append array binary break catch cd concat continue error eval exec exit expr file flush for foreach format global if incr info join lappend lassign lindex linsert list llength lrange lreplace lsearch lset lsort namespace open proc puts read regexp regsub rename return scan seek set socket source split string subst switch trace unset uplevel upvar variable while",
            "",
            "true false",
        )
        .line(HASH)
        .strings("\"", "")
        .sigils("$"),
    Dialect::new("makefile", "makefile make mk mak bsdmake gnumake")
        .words(
            "ifeq ifneq ifdef ifndef else endif include sinclude export unexport override define endef vpath wildcard patsubst subst shell foreach call eval notdir basename addprefix",
            "",
            "",
        )
        .line(HASH)
        .strings("\"'", "")
        .sigils("$")
        .flags(KEYS),
    Dialect::new("json", "json jsonc json5")
        .words("", "", "true false null")
        .strings("\"", "")
        .flags(KEYS),
    Dialect::new("yaml", "yaml yml")
        .words("", "", "true false null yes no on off Yes No True False Null")
        .line(HASH)
        .strings("\"'", "")
        .flags(KEYS),
    Dialect::new("toml", "toml tml cargo")
        .words("", "", "true false")
        .line(HASH)
        .strings("\"'", "")
        .flags(KEYS | TABLE | TRIPLE),
    Dialect::new("git", "git gitconfig gitcommit gitignore")
        .words("pick reword edit squash fixup drop", "", "true false")
        .line(HASH)
        .strings("\"", "")
        .flags(KEYS | TABLE | DIFF),
    Dialect::new("diff", "diff patch udiff").flags(DIFF),
    Dialect::new("dot", "dot graphviz gv neato")
        .words(
            "digraph graph subgraph node edge strict rank rankdir cluster",
            "",
            "true false",
        )
        .line(&["//", "#"])
        .block(CBLOCK)
        .strings("\"", "")
        .flags(KEYS),
    Dialect::new("css", "css scss sass less stylesheet")
        .words(
            "media import keyframes font-face supports charset namespace page use include mixin extend and not only from to important",
            "px em rem ex ch vh vw vmin vmax pt pc cm mm in deg rad turn fr",
            "inherit initial unset revert none auto transparent currentColor",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "")
        .sigils("$")
        .word("-")
        .flags(KEYS | DECOR),
    Dialect::new("html", "html htm xhtml")
        .words("", "", "")
        .block(SGML)
        .strings("\"'", "")
        .flags(MARKUP | EMBED),
    Dialect::new("xml", "xml xsd xsl xslt svg rss atom plist xaml")
        .words("", "", "")
        .block(SGML)
        .strings("\"'", "")
        .flags(MARKUP),
    Dialect::new("markdown", "markdown md mdown mkd mkdn").flags(MD),
    Dialect::new("rst", "rst rest restructuredtext").flags(RST),
    Dialect::new("textile", "textile").flags(TEXTILE),
    Dialect::new("latex", "latex tex context sty cls bibtex")
        .line(&["%"])
        .flags(TEX),
    Dialect::new("regex", "regex regexp re pcre").flags(REGEX),
    Dialect::new("pascal", "pascal delphi pas objectpascal dpr")
        .words(
            "and array asm begin case const constructor destructor div do downto else end except file finally for function goto if implementation in inherited initialization inline interface label mod nil not object of or packed procedure program property raise record repeat set shl shr then to try type unit until uses var while with xor",
            "integer real boolean char string byte word longint shortint cardinal single double extended pointer array",
            "true false nil",
        )
        .line(SLASH)
        .block(&[("(*", "*)"), ("{", "}")])
        .strings("'", "")
        .flags(ICASE),
    Dialect::new("objective-c", "objective-c objectivec objc obj-c mm")
        .words(
            "interface implementation protocol property synthesize dynamic end selector encode class autoreleasepool try catch finally throw synchronized break case continue default do else for goto if return sizeof switch typedef while static const extern inline struct enum union import include",
            "NSString NSArray NSMutableArray NSDictionary NSMutableDictionary NSNumber NSInteger NSUInteger NSObject NSError CGFloat CGRect BOOL id instancetype void char int float double long short unsigned signed",
            "nil Nil YES NO NULL self super",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", "@")
        .flags(CHAR | PREPROC | DECOR),
    Dialect::new("d", "d dlang")
        .words(
            "abstract alias align asm assert auto body break case cast catch class const continue debug default delegate delete deprecated do else enum export extern final finally for foreach foreach_reverse function goto if immutable import in inout interface invariant is lazy mixin module new nothrow out override package pragma private protected public pure ref return scope shared static struct super switch synchronized template this throw try typeid typeof union unittest version while with",
            "bool byte ubyte short ushort int uint long ulong cent ucent float double real char wchar dchar string wstring dstring size_t void",
            "true false null",
        )
        .line(SLASH)
        .block(&[("/*", "*/"), ("/+", "+/")])
        .strings("\"'`", "r")
        .flags(NEST | CHAR),
    Dialect::new("actionscript", "actionscript as3 flash swf")
        .words(
            "package import public private protected internal class interface extends implements function var const if else for each in while do switch case break continue return new delete typeof instanceof try catch finally throw super this dynamic final override static get set namespace use trace",
            "int uint Number String Boolean Array Object Vector void Function Event Sprite MovieClip",
            "true false null undefined NaN",
        )
        .line(SLASH)
        .block(CBLOCK)
        .strings("\"'", ""),
    Dialect::new("applescript", "applescript scpt osascript")
        .words(
            "tell end set to of on run if then else repeat while until try error return script property global local considering ignoring with without my its me activate delay copy get make new count exists using terms from application",
            "text integer real list record boolean alias date file",
            "true false missing value it result",
        )
        .line(&["--", "#"])
        .block(PAREN)
        .strings("\"", "")
        .flags(ICASE),
    Dialect::new("asp", "asp aspx vbscript vb classic-asp")
        .words(
            "dim set if then elseif else end select case for each next do loop while wend function sub call redim option explicit on error resume class public private const exit byval byref new with preserve and or not mod is response request server session application",
            "string integer long double boolean variant object date byte",
            "true false nothing null empty vbCrLf",
        )
        .line(&["'", "rem"])
        .strings("\"", "")
        .flags(ICASE | MARKUP | EMBED),
];
