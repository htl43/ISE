use electron_sys::ipc_renderer;
use pest::Parser;
use std::collections::HashMap;
use std::fs;
use std::num::NonZeroU32;
use std::ops::Deref;
use std::option::Option;
use stdweb::unstable::TryInto;
use stdweb::web::{document, IElement, IHtmlElement, INode, IParentNode};
use wasm_bindgen::JsValue;
use yew::events::{ClickEvent, IKeyboardEvent, KeyPressEvent};
use yew::prelude::*;
use yew::services::reader::{File, FileData, ReaderService, ReaderTask};
use yew::services::ConsoleService;

use crate::coordinate::{Col, Coordinate, Row};
use crate::grammar::{Grammar, Interactive, Kind, Lookup};
use crate::session::Session;
use crate::style::Style;
use crate::util::{apply_definition_grammar, dom_resize, move_grammar, non_zero_u32_tuple, resize};
use crate::view::{view_grammar, view_menu_bar, view_side_nav, view_tab_bar};
use crate::{coord, coord_col, coord_row, row_col_vec};

#[derive(Parser)]
#[grammar = "coordinate.pest"]
pub struct CoordinateParser;

// Model contains the entire state of the application
#[derive(Debug)]
pub struct Model {
    view_root: Coordinate,
    pub first_select_cell: Option<Coordinate>,
    pub last_select_cell: Option<Coordinate>,
    pub active_cell: Option<Coordinate>,
    pub suggestions: Vec<Coordinate>,
    pub col_widths: HashMap<Col, f64>,
    pub row_heights: HashMap<Row, f64>,
    pub select_grammar: Vec<Coordinate>,
    pub sessions: Vec<Session>,
    pub current_session_index: usize,
    pub side_menus: Vec<SideMenu>,
    pub open_side_menu: Option<i32>,
    pub focus_node_ref: NodeRef,
    pub link: ComponentLink<Model>,
    console: ConsoleService,
    reader: ReaderService,
    tasks: Vec<ReaderTask>,
}

#[derive(Debug)]
pub struct SideMenu {
    pub name: String,
    pub icon_path: String,
}

// ACTIONS
// Triggered in the view, sent to update function
pub enum Action {
    // Do nothing
    Noop,

    // Change string value of Input grammar
    ChangeInput(Coordinate, /* new_value: */ String),

    SetActiveCell(Coordinate),

    DoCompletion(
        /* source: */ Coordinate,
        /* destination */ Coordinate,
    ),

    SetActiveMenu(Option<i32>),

    ReadSession(/* filename: */ File),

    LoadSession(FileData),

    SaveSession(),

    SetSessionTitle(String),
    ReadDriverFiles(Vec<File>),
    LoadDriverMainFile(FileData),
    UploadDriverMiscFile(FileData),

    // Grid Operations
    AddNestedGrid(Coordinate, (u32 /*rows*/, u32 /*cols*/)),

    InsertRow,
    InsertCol,

    // Alerts and stuff
    Alert(String),

    SetSelectedCells(Coordinate),
    Lookup(
        /* source: */ Coordinate,
        /* lookup_type: */ Lookup,
    ),

    ToggleLookup(Coordinate),

    DefnUpdateName(Coordinate, /* name */ String),
    DefnUpdateRule(Coordinate, /* rule Row  */ Row),
    DefnAddRule(Coordinate), // adds a new column, points rule coordinate to bottom of ~meta~ sub-table
                             // Definition Rules are represented as grammars
}

impl Model {
    pub fn get_session(&self) -> &Session {
        &self.sessions[self.current_session_index]
    }

    pub fn get_session_mut(&mut self) -> &mut Session {
        &mut self.sessions[self.current_session_index]
    }

    // only use this if you need a COPY of the current session
    // i.e. not changing its values
    pub fn to_session(&self) -> Session {
        self.get_session().clone()
    }

    fn load_session(&mut self, session: Session) {
        self.get_session_mut().root = session.root;
        self.get_session_mut().meta = session.meta;
        self.get_session_mut().grammars = session.grammars;
    }

    fn query_parent(&self, coord_parent: Coordinate) -> Vec<Coordinate> {
        self.get_session()
            .grammars
            .keys()
            .clone()
            .filter_map(|k| {
                if k.parent() == Some(coord_parent.clone()) {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn query_col(&self, coord_col: Col) -> Vec<Coordinate> {
        self.get_session()
            .grammars
            .keys()
            .clone()
            .filter_map(|k| {
                if k.row_cols.len() == 1
                /* ignore root & meta */
                {
                    None
                } else if k.full_col() == coord_col {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    fn query_row(&self, coord_row: Row) -> Vec<Coordinate> {
        self.get_session()
            .grammars
            .keys()
            .clone()
            .filter_map(|k| {
                if k.row_cols.len() == 1
                /* ignore root & meta */
                {
                    None
                } else if k.full_row() == coord_row {
                    Some(k.clone())
                } else {
                    None
                }
            })
            .collect()
    }
}

impl Component for Model {
    type Message = Action;
    type Properties = ();

    fn create(_: Self::Properties, link: ComponentLink<Self>) -> Self {
        let root_grammar = Grammar {
            name: "root".to_string(),
            style: Style::default(),
            kind: Kind::Grid(row_col_vec![(1, 1), (2, 1), (3, 1), (1, 2), (2, 2), (3, 2)]),
        };
        let meta_grammar = Grammar {
            name: "meta".to_string(),
            style: Style::default(),
            kind: Kind::Grid(row_col_vec![(1, 1), (2, 1), (3, 1)]),
        };
        let mut m = Model {
            view_root: coord!("root"),
            col_widths: hashmap! {
               coord_col!("root","A") => 90.0,
               coord_col!("root","B") => 90.0,
               coord_col!("meta","A") => 180.0,
               coord_col!("meta-A3","A") => 90.0,
               coord_col!("meta-A3","B") => 180.0,
            },
            row_heights: hashmap! {
               coord_row!("root","1") => 30.0,
               coord_row!("root","2") => 30.0,
               coord_row!("root","3") => 30.0,
               coord_row!("meta","1") => 180.0,
            },
            active_cell: Some(coord!("root-A1")),
            suggestions: vec![coord!("meta-A1"), coord!("meta-A2"), coord!("meta-A3")],

            console: ConsoleService::new(),
            reader: ReaderService::new(),

            select_grammar: vec![],
            first_select_cell: None,
            last_select_cell: None,

            sessions: vec![Session {
                title: "my session".to_string(),
                root: root_grammar.clone(),
                meta: meta_grammar.clone(),
                grammars: hashmap! {
                    coord!("root")    => root_grammar.clone(),
                    coord!("root-A1") => Grammar::default(),
                    coord!("root-A2") => Grammar::default(),
                    coord!("root-A3") => Grammar::default(),
                    coord!("root-B1") => Grammar::default(),
                    coord!("root-B2") => Grammar::default(),
                    coord!("root-B3") => Grammar::default(),
                    coord!("meta")    => meta_grammar.clone(),
                    coord!("meta-A1") => Grammar::text("js grammar".to_string(), "This is js".to_string()),
                    coord!("meta-A2") => Grammar::text("java grammar".to_string(), "This is java".to_string()),
                    coord!("meta-A3") => Grammar {
                        name: "defn".to_string(),
                        style: Style::default(),
                        kind: Kind::Defn(
                            "".to_string(),
                            coord!("meta-A3"),
                            vec![
                                ("".to_string(), coord!("meta-A3-B1")),
                            ],
                        ),
                    },
                    coord!("meta-A3-A1")    => Grammar::default(),
                    coord!("meta-A3-B1")    => Grammar {
                        name: "root".to_string(),
                        style: Style::default(),
                        kind: Kind::Grid(row_col_vec![ (1,1), (2,1), (1,2), (2,2) ]),
                    },
                    coord!("meta-A3-B1-A1") => Grammar::text("".to_string(), "custom grammar".to_string()),
                    coord!("meta-A3-B1-A2") => Grammar::default(),
                    coord!("meta-A3-B1-B1") => Grammar::default(),
                    coord!("meta-A3-B1-B2") => Grammar::default(),
                },
            }],

            current_session_index: 0,

            side_menus: vec![
                SideMenu {
                    name: "Home".to_string(),
                    icon_path: "assets/logo.png".to_string(),
                },
                SideMenu {
                    name: "File Explorer".to_string(),
                    icon_path: "assets/folder_icon.png".to_string(),
                },
                SideMenu {
                    name: "Settings".to_string(),
                    icon_path: "assets/settings_icon.png".to_string(),
                },
                SideMenu {
                    name: "Info".to_string(),
                    icon_path: "assets/info_icon.png".to_string(),
                },
            ],
            open_side_menu: None,

            link,
            tasks: vec![],

            focus_node_ref: NodeRef::default(),
        };
        // apply_definition_grammar(&mut m, coord!("meta-A3"));
        m
    }

    // The update function is split into sub-update functions that
    // are specifc to each EventType
    fn update(&mut self, event_type: Self::Message) -> ShouldRender {
        match event_type {
            Action::Noop => false,

            Action::Alert(message) => {
                self.console.log(&message);
                // TODO: make this into a more visual thing
                false
            }

            Action::ChangeInput(coord, new_value) => {
                if let Some(g) = self.get_session_mut().grammars.get_mut(&coord) {
                    match g {
                        Grammar {
                            kind: Kind::Input(_),
                            ..
                        } => {
                            info!("{}", &new_value);
                            g.kind = Kind::Input(new_value);
                        }
                        Grammar {
                            kind: Kind::Lookup(_, lookup_type),
                            ..
                        } => {
                            info!("{}", &new_value);
                            g.kind = Kind::Lookup(new_value, lookup_type.clone());
                        }
                        _ => (),
                    }
                }
                false
            }

            Action::SetActiveCell(coord) => {
                self.first_select_cell = Some(coord.clone());
                self.last_select_cell = None;
                self.active_cell = Some(coord.clone());
                true
            }

            Action::SetSelectedCells(coord) => {
                self.last_select_cell = Some(coord.clone());
                true
            }

            Action::DoCompletion(source_coord, dest_coord) => {
                move_grammar(
                    &mut self.get_session_mut().grammars,
                    source_coord,
                    dest_coord.clone(),
                );
                let row_height = self.row_heights.get(&dest_coord.full_row()).unwrap();
                let col_width = self.col_widths.get(&dest_coord.full_col()).unwrap();
                resize(self, dest_coord, *row_height, *col_width);
                true
            }

            Action::SetActiveMenu(active_menu) => {
                self.open_side_menu = active_menu;
                true
            }

            Action::ReadSession(file) => {
                let callback = self.link.callback(Action::LoadSession);
                let task = self.reader.read_file(file, callback);
                self.tasks.push(task);
                false
            }

            Action::LoadSession(file_data) => {
                let session: Session =
                    serde_json::from_str(format! {"{:?}", file_data}.deref()).unwrap();
                self.load_session(session);
                true
            }

            Action::SaveSession() => {
                /* TODO: uncomment when this is working
                use node_sys::fs as node_fs;
                use node_sys::Buffer;
                use js_sys::{
                    JsString,
                    Function
                };
                let session = self.to_session();
                let j = serde_json::to_string(&session.clone());
                let filename = session.title.to_string();
                let jsfilename = JsString::from(filename);
                let jsbuffer = Buffer::from_string(&JsString::from(j.unwrap()), None);
                let jscallback = Function::new_no_args("{}");
                node_fs::append_file(&jsfilename, &jsbuffer, None, &jscallback);
                */
                false
            }

            Action::SetSessionTitle(name) => {
                self.get_session_mut().title = name;
                true
            }

            Action::ReadDriverFiles(files_list) => {
                // Get the main file and miscellaneous/additional files from the drivers list
                let (main_file, misc_files) = {
                    let (main_file_as_vec, misc_files) : (Vec<File>, Vec<File>) = files_list.iter().fold((Vec::new(), Vec::new()), |accum, file| { 
                        // Iter::partition is used to divide a list into two given a certain condition.
                        // Here the condition here is to separate the main file of the driver from the
                        // addditional ones, where the main file's path looks like
                        // '{directory_name}/{file_name}.js' and the directory_name == file_name
                        let mut new_accum = accum.clone();
                        // use `webkitRelativePath` as the `name`, if it's available
                        // we'll call out to regular JS to do this using the `js!` macro.
                        // Note that yew::services::reader::File::name() calls "file.name" under the
                        // hood (https://docs.rs/stdweb/0.4.20/src/stdweb/webapi/file.rs.html#23)
                        let full_file_name : String = js!(
                            if (!!@{&file}.webkitRelativePath) {
                                return @{&file}.webkitRelativePath;
                            } else {
                                console.log("couldn't get relative path of file: ", @{&file}.name);
                                return @{&file}.name; // equivalent to yew::services::reader::File::name()
                            }
                        ).try_into().unwrap();
                        let path_parts : Vec<&str> = full_file_name.split("/").collect();
                        match (path_parts.first(), path_parts.last(), path_parts.len()) {
                            (Some(directory), Some(file_name), 2) if format!{"{}.js", directory} == file_name.to_string() => {
                                if new_accum.0.len() == 0 {
                                    new_accum.0.push(file.clone());
                                } else {
                                    panic!("[Action::ReadDriverFiles]: there shouldn't be more than 1 main file in the driver directory")
                                }
                            },
                            _ => {
                                new_accum.1.push(file.clone());
                            },
                        };
                        new_accum
                    });
                    // the `partition` call above gives us a tuple of two Vecs (Vec, Vec) where the first Vec
                    // should have only one element, so we'll convert it to a (Vec::Item, Vec).
                    // If this has an error, then there's something wrong with how the driver
                    // directory is organized.
                    (
                        main_file_as_vec.first().unwrap().clone(),
                        misc_files.clone(),
                    )
                };

                // upload misc files so they can be served by electron to be used by main driver file
                let upload_callback = self
                    .link
                    .callback(|file_data| Action::UploadDriverMiscFile(file_data));
                for file in misc_files {
                    let task = self.reader.read_file(file, upload_callback.clone());
                    self.tasks.push(task);
                }

                // Load main driver file. After this task has been scheduled and executed, the
                // driver is ready for use.
                self.tasks.push(
                    self.reader
                        .read_file(main_file, self.link.callback(Action::LoadDriverMainFile)),
                );

                false
            }

            Action::UploadDriverMiscFile(file_data) => {
                // Here, we use some electron APIs to call out to the main process in JS.
                // For this, we use the `electron_sys` library which is pretty experimental but
                // feature complete.
                // See here for documentation how to communicate between the main and renderer proess in Electron:
                //   https://www.tutorialspoint.com/electron/electron_inter_process_communication.htm
                // And here, for the documentation for the electon_sys Rust bindings for electron.ipcRenderer:
                //   https://docs.rs/electron-sys/0.4.0/electron_sys/struct.IpcRenderer.html

                let args: [JsValue; 2] = [
                    JsValue::from_str(file_data.name.deref()),
                    JsValue::from_str(std::str::from_utf8(&file_data.content).unwrap()),
                ];
                ipc_renderer.send_sync("upload-driver-misc-file", Box::new(args));
                false
            }

            Action::LoadDriverMainFile(main_file_data) => {
                info! {"Loading Driver: {}", &main_file_data.name};
                let file_contents = std::str::from_utf8(&main_file_data.content).unwrap();
                // dump file contents into script tag and attach to the DOM
                let script = document().create_element("script").unwrap();
                script.set_text_content(file_contents);
                script.set_attribute("type", "text/javascript");
                script.set_attribute("class", "ise-driver");
                script.set_attribute("defer", "true");
                let head = document().query_selector("head").unwrap().unwrap();
                head.append_child(&script);
                true
            }

            Action::AddNestedGrid(coord, (rows, cols)) => {
                // height and width initial values
                let mut tmp_heigt = 30.0;
                let mut tmp_width = 90.0;
                let (r, c) = non_zero_u32_tuple((rows, cols));
                let grammar = Grammar::as_grid(r, c);
                if let Kind::Grid(sub_coords) = grammar.clone().kind {
                    self.active_cell = sub_coords.first().map(|c| Coordinate::child_of(&coord, *c));
                    // let row_val = coord

                    let current_width = self.col_widths[&coord.full_col()];
                    let current_height = self.row_heights[&coord.full_row()];

                    // check if active cell row height and width is greater than default value
                    if current_width > tmp_width {
                        // set height argument to active cell height if greater
                        //Get the actual amount of cell being created and use it instead of "3" being HARD CODED.
                        tmp_width = current_width / 3.0;
                    }
                    if current_height > tmp_heigt {
                        // set width argument to active cell width if greater
                        //Get the actual amount of cell being created and use it instead of "3" being HARD CODED.
                        tmp_heigt = current_height / 3.0;
                    }

                    for sub_coord in sub_coords {
                        let new_coord = Coordinate::child_of(&coord, sub_coord);
                        self.get_session_mut()
                            .grammars
                            .insert(new_coord.clone(), Grammar::default());
                        // initialize row & col heights as well
                        if !self.row_heights.contains_key(&new_coord.clone().full_row()) {
                            self.row_heights
                                .insert(new_coord.clone().full_row(), tmp_heigt);
                            //30.0);
                        }
                        if !self.col_widths.contains_key(&new_coord.clone().full_col()) {
                            self.col_widths
                                .insert(new_coord.clone().full_col(), tmp_width);
                            //90.0);
                        }
                    }
                }
                if let Some(parent) = Coordinate::parent(&coord)
                    .and_then(|p| self.get_session_mut().grammars.get_mut(&p))
                {
                    parent.kind = grammar.clone().kind; // make sure the parent gets set to Kind::Grid
                }
                self.get_session_mut()
                    .grammars
                    .insert(coord.clone(), grammar);
                resize(
                    self,
                    coord,
                    (rows as f64) * (/* default row height */30.0),
                    (cols as f64) * (/* default col width */90.0),
                );
                true
            }
            Action::InsertCol => {
                if let Some(coord) = self.active_cell.clone() {
                    // find the bottom-most coord
                    let mut right_most_coord = coord.clone();
                    while let Some(right_coord) = right_most_coord.neighbor_right() {
                        if self.to_session().grammars.contains_key(&right_coord) {
                            right_most_coord = right_coord;
                        } else {
                            break;
                        }
                    }

                    let right_most_col_coords = self.query_col(right_most_coord.full_col());
                    let new_col_coords = right_most_col_coords
                        .iter()
                        .map(|c| (c.row(), NonZeroU32::new(c.col().get() + 1).unwrap()));

                    let parent = coord.parent().unwrap();
                    if let Some(Grammar {
                        kind: Kind::Grid(sub_coords),
                        name,
                        style,
                    }) = self.get_session().grammars.get(&parent)
                    {
                        let mut new_sub_coords = sub_coords.clone();
                        let mut grammars = self.to_session().grammars.clone();
                        for c in new_col_coords {
                            grammars.insert(
                                Coordinate::child_of(&parent.clone(), c),
                                Grammar::default(),
                            );
                            new_sub_coords.push(c);
                        }
                        grammars.insert(
                            parent,
                            Grammar {
                                kind: Kind::Grid(new_sub_coords.clone()),
                                name: name.clone(),
                                style: style.clone(),
                            },
                        );
                        self.get_session_mut().grammars = grammars;
                    }
                }
                true
            }
            Action::InsertRow => {
                if let Some(coord) = self.active_cell.clone() {
                    // find the bottom-most coord
                    let mut bottom_most_coord = coord.clone();
                    while let Some(below_coord) = bottom_most_coord.neighbor_below() {
                        if self.get_session().grammars.contains_key(&below_coord) {
                            bottom_most_coord = below_coord;
                        } else {
                            break;
                        }
                    }

                    let bottom_most_row_coords = self.query_row(bottom_most_coord.full_row());
                    let new_row_coords = bottom_most_row_coords
                        .iter()
                        .map(|c| (NonZeroU32::new(c.row().get() + 1).unwrap(), c.col()));

                    let parent = coord.parent().unwrap();
                    if let Some(Grammar {
                        kind: Kind::Grid(sub_coords),
                        name,
                        style,
                    }) = self.get_session().grammars.get(&parent)
                    {
                        let mut new_sub_coords = sub_coords.clone();
                        let mut grammars = self.get_session().grammars.clone();
                        for c in new_row_coords {
                            grammars.insert(
                                Coordinate::child_of(&parent.clone(), c),
                                Grammar::default(),
                            );
                            new_sub_coords.push(c);
                        }
                        grammars.insert(
                            parent,
                            Grammar {
                                kind: Kind::Grid(new_sub_coords.clone()),
                                name: name.clone(),
                                style: style.clone(),
                            },
                        );
                        self.get_session_mut().grammars = grammars;
                    }
                }
                true
            }
            Action::Lookup(source_coord, lookup_type) => {
                match lookup_type {
                    Lookup::Cell(dest_coord) => {
                        move_grammar(
                            &mut self.get_session_mut().grammars,
                            source_coord,
                            dest_coord.clone(),
                        );
                    }
                    _ => (),
                }
                false
            }

            Action::ToggleLookup(coord) => {
                match self.get_session_mut().grammars.get_mut(&coord) {
                    Some(
                        g @ Grammar {
                            kind: Kind::Input(_),
                            ..
                        },
                    ) => {
                        g.kind = Kind::Lookup("".to_string(), None);
                    }
                    Some(
                        g @ Grammar {
                            kind: Kind::Lookup(_, _),
                            ..
                        },
                    ) => {
                        g.kind = Kind::Input("".to_string());
                    }
                    _ => {
                        info! { "[Action::ToggleLookup] cannot toggle non-Input/Lookup kind of grammar" }
                    }
                };
                true
            }

            Action::DefnUpdateName(coord, name) => false,
            Action::DefnUpdateRule(coord, rule_row) => false,
            Action::DefnAddRule(coord) => {
                // TODO adds a new column, points rule coordinate to bottom of ~meta~ sub-table
                false
            }
        }
    }

    fn view(&self) -> Html {
        let active_cell = self.active_cell.clone();
        html! {
            <div>

                { view_side_nav(&self) }

                { view_menu_bar(&self) }

                { view_tab_bar(&self) }

                <div class="main">
                    <div id="grammars" class="grid-wrapper" onkeypress=self.link.callback(move |e : KeyPressEvent| {
                        // Global Key-Shortcuts
                        Action::Noop
                    })>
                        { view_grammar(&self, coord!{"root"}) }
                    </div>
                </div>
            </div>
        }
    }
}
