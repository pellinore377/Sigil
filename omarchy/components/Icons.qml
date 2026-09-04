// Generated from `shared/icons.json` by `shared/icongen`. Do not hand-edit.
// Add or change an icon there, then re-run the generator.

pragma Singleton
import QtQuick

// Every icon in Sigil, by what it means here rather than what the icon set calls it.
// Call sites say `Icons.back`, never a glyph literal. Draw them with `IconLabel`.
// QML has no `font.families`: an element names ONE family, so an icon cannot share a
// string with words — split it into an IconLabel plus a Text.
// These are private-use codepoints that Nerd Fonts also claim, so the wrong family
// silently draws a DIFFERENT icon rather than tofu.
// Trailing comments are the canonical Material Symbols names: fonts.google.com/icons.
QtObject {

  // navigation
  readonly property string back:           "\uE5C4"  // arrow_back
  readonly property string close:          "\uE5CD"  // close
  readonly property string chevronDown:    "\uE5CF"  // expand_more
  readonly property string chevronUp:      "\uE5CE"  // expand_less
  readonly property string chevronRight:   "\uE5CC"  // chevron_right
  readonly property string arrowDown:      "\uE5DB"  // arrow_downward
  readonly property string arrowUp:        "\uE5D8"  // arrow_upward
  readonly property string home:           "\uE9B2"  // home
  readonly property string maximize:       "\uE5D0"  // fullscreen
  readonly property string fullscreen:     "\uE5D0"  // fullscreen
  readonly property string windowed:       "\uE8AA"  // picture_in_picture
  readonly property string pip:            "\uF64D"  // pip
  readonly property string statusRing:     "\uEEE1"  // circle_circle
  readonly property string statusDot:      "\uEF4A"  // circle
  readonly property string checkCircle:    "\uE86C"  // check_circle
  readonly property string errorMark:      "\uE000"  // error
  readonly property string collapse:       "\uF1CF"  // close_fullscreen

  // action
  readonly property string plus:           "\uE145"  // add
  readonly property string plusCircle:     "\uE990"  // add_circle
  readonly property string cancel:         "\uE888"  // cancel
  readonly property string check:          "\uE668"  // check
  readonly property string search:         "\uEF7A"  // search
  readonly property string edit:           "\uF097"  // edit
  readonly property string copy:           "\uE14D"  // content_copy
  readonly property string cut:            "\uE14E"  // content_cut
  readonly property string paste:          "\uE14F"  // content_paste
  readonly property string selectAll:      "\uE162"  // select_all
  readonly property string trash:          "\uE92E"  // delete
  readonly property string reply:          "\uE15E"  // reply
  readonly property string replyArrow:     "\uE15E"  // reply
  readonly property string retry:          "\uE5D5"  // refresh
  readonly property string forward:        "\uE154"  // forward
  readonly property string share:          "\uE80D"  // share
  readonly property string link:           "\uE250"  // link
  readonly property string download:       "\uF090"  // download
  readonly property string attach:         "\uE226"  // attach_file
  readonly property string send:           "\uE163"  // send
  readonly property string react:          "\uE1D3"  // add_reaction
  readonly property string refresh:        "\uE5D5"  // refresh
  readonly property string stop:           "\uE047"  // stop
  readonly property string settings:       "\uE8B8"  // settings
  readonly property string moreVertical:   "\uE5D4"  // more_vert
  readonly property string moreHorizontal: "\uE5D3"  // more_horiz
  readonly property string spinner:        "\uE9D0"  // progress_activity

  // content
  readonly property string chat:           "\uE0C9"  // chat
  readonly property string thread:         "\uE0B9"  // comment
  readonly property string pin:            "\uF10D"  // push_pin
  readonly property string keep:           "\uE6AA"  // keep
  readonly property string file:           "\uE873"  // description
  readonly property string folder:         "\uE2C7"  // folder
  readonly property string camera:         "\uE412"  // photo_camera
  readonly property string codeBlocks:     "\uF84D"  // code_blocks
  readonly property string image:          "\uE3F4"  // image
  readonly property string voiceMemo:      "\uE1B8"  // graphic_eq
  readonly property string music:          "\uE405"  // music_note
  readonly property string audioNote:      "\uEB82"  // audio_file
  readonly property string sticker:        "\uE707"  // sticker
  readonly property string emoji:          "\uEA22"  // mood
  readonly property string poll:           "\uE172"  // ballot
  readonly property string note:           "\uE26C"  // notes
  readonly property string email:          "\uE159"  // mail
  readonly property string palette:        "\uE40A"  // palette
  readonly property string logout:         "\uE9BA"  // logout
  readonly property string space:          "\uEA0F"  // workspaces
  readonly property string lowPriority:    "\uE16D"  // low_priority
  readonly property string emojiMore:      "\uE1D3"  // add_reaction

  // people
  readonly property string people:         "\uEA21"  // group
  readonly property string person:         "\uE7FD"  // person
  readonly property string personAdd:      "\uEA4D"  // person_add

  // media
  readonly property string play:           "\uE037"  // play_arrow
  readonly property string pause:          "\uE034"  // pause

  // call
  readonly property string phone:          "\uE0B0"  // call
  readonly property string callEnd:        "\uF0BC"  // call_end
  readonly property string micOn:          "\uE31D"  // mic
  readonly property string record:         "\uE029"  // mic
  readonly property string micOff:         "\uE02B"  // mic_off
  readonly property string videoOn:        "\uE04B"  // videocam
  readonly property string videoOff:       "\uE04C"  // videocam_off
  readonly property string screenShare:    "\uE0E2"  // screen_share
  readonly property string screenShareAlt: "\uE0DF"  // present_to_all
  readonly property string speaker:        "\uE050"  // volume_up
  readonly property string signalOff:      "\uF239"  // signal_disconnected

  // location
  readonly property string location:       "\uF1DB"  // place
  readonly property string liveLocation:   "\uF05F"  // share_location
  readonly property string myLocation:     "\uE55C"  // my_location
  readonly property string pinDrop:        "\uE55E"  // pin_drop
  readonly property string recentre:       "\uE55C"  // my_location

  // security
  readonly property string lock:           "\uE897"  // lock
  readonly property string lockOff:        "\uE898"  // lock_open
  readonly property string recoveryKey:    "\uE73C"  // key
  readonly property string login:          "\uEA77"  // login
  readonly property string eye:            "\uE8F4"  // visibility
  readonly property string eyeOff:         "\uE8F5"  // visibility_off

  // status
  readonly property string clock:          "\uEFD6"  // schedule
  readonly property string alert:          "\uF083"  // warning
  readonly property string errorCircle:    "\uF8B6"  // error

  // spaces and settings
  readonly property string globe:          "\uE80B"  // public
  readonly property string shield:         "\uE9E0"  // shield
  readonly property string moderator:      "\uF7AC"  // chat_error
  readonly property string leave:          "\uE9BA"  // logout
  readonly property string hash:           "\uE9EF"  // tag
  readonly property string star:           "\uE838"  // star
  readonly property string bell:           "\uE7F4"  // notifications
  readonly property string bellOff:        "\uE7F6"  // notifications_off

  // emoji categories
  readonly property string emojiPeople:    "\uEA1D"  // emoji_people
  readonly property string emojiNature:    "\uEA1C"  // emoji_nature
  readonly property string emojiFood:      "\uEA1B"  // emoji_food_beverage
  readonly property string emojiTravel:    "\uEA1F"  // emoji_transportation
  readonly property string emojiEvents:    "\uE71A"  // trophy
  readonly property string emojiObjects:   "\uEA24"  // emoji_objects
  readonly property string emojiSymbols:   "\uEA1E"  // emoji_symbols
  readonly property string emojiFlags:     "\uE153"  // flag
}
