-file("src/absinthe_parser.yrl", 0).
-module(absinthe_parser).
-file("src/absinthe_parser.erl", 3).
-export([parse/1, parse_and_scan/1, format_error/1]).
-file("src/absinthe_parser.yrl", 376).

% Line-Level Utilities

extract_location({_Token, {Line, Column}}) ->
  #{'line' => Line, 'column' => Column};
extract_location({_Token, {Line, Column}, _Value}) ->
  #{'line' => Line, 'column' => Column};
extract_location(_Other) ->
  #{'line' => nil, 'column' => nil}.

extract_child_location([Head|_]) ->
  extract_child_location(Head);
extract_child_location(#{loc := #{'line' := Line, 'column' := Column}}) ->
  #{'line' => Line, 'column' => Column};
extract_child_location(_) ->
  #{'line' => nil, 'column' => nil}.

% Value-level Utilities

extract_atom({Value, _Loc}) ->
  Value.

extract_binary(Value) when is_binary(Value) ->
  Value;

extract_binary({Token, _Loc}) ->
  list_to_binary(atom_to_list(Token));

extract_binary({_Token, _Loc, Value}) ->
  list_to_binary(Value).

% AST Generation

build_ast_node(Type, Node, #{'line' := nil, 'column' := nil}) ->
  build_ast_node(Type, Node, nil);
build_ast_node(Type, Node, Loc) ->
  'Elixir.Kernel':struct(list_to_atom("Elixir.Absinthe.Language." ++ atom_to_list(Type)), Node#{loc => Loc}).

% Descriptions

put_description(Node, Description) ->
  maps:put(description, Description, Node).

% String

extract_quoted_string_token({_Token, _Loc, Value}) ->
  unicode:characters_to_binary(lists:sublist(Value, 2, length(Value) - 2)).

% Block String

extract_quoted_block_string_token({_Token, _Loc, Value}) ->
  unicode:characters_to_binary(process_block_string(lists:sublist(Value, 4, length(Value) - 6))).

-spec process_block_string(string()) -> string().
process_block_string(Escaped) ->
  process_block_string(Escaped, []).

-spec process_block_string(string(), string()) -> string().
process_block_string([], Acc) ->
  block_string_value(lists:reverse(Acc));
process_block_string([$\r, $\n | T], Acc) -> process_block_string(T, [$\n | Acc]);
process_block_string([$\\, $", $", $" | T], Acc) -> process_block_string(T, [$", $", $"] ++ Acc);
process_block_string([H | T], Acc) -> process_block_string(T, [H | Acc]).

-spec block_string_value(string()) -> string().
block_string_value(Value) ->
  [FirstLine | Rest] = string:split(Value, "\n", all),
  Prefix = indentation_prefix(common_indent(Rest)),
  UnindentedLines = unindent(Rest, Prefix),
  Lines = trim_blank_lines([FirstLine | UnindentedLines]),
  string:join(Lines, "\n").

-spec trim_blank_lines([string()]) -> [string()].
trim_blank_lines(Lines) ->
  trim_blank_lines(trim_blank_lines(Lines, leading), trailing).

-spec trim_blank_lines([string()], leading | trailing) -> [string()].
trim_blank_lines(Lines, leading) ->
  lists:dropwhile(fun is_blank/1, Lines);
trim_blank_lines(Lines, trailing) ->
  lists:reverse(trim_blank_lines(lists:reverse(Lines), leading)).

-spec indentation_prefix(non_neg_integer()) -> string().
indentation_prefix(Indent) ->
  lists:map(fun(_) -> 32 end, lists:seq(1, Indent)).

-spec unindent([string()], string()) -> [string()].
unindent(Lines, Prefix) ->
  unindent(Lines, Prefix, []).

-spec unindent([string()], string(), [string()]) -> [string()].
unindent([], _Prefix, Result) ->
  lists:reverse(Result);
unindent([H | T], Prefix, Result) ->
  Processed = prefix(H, Prefix),
  unindent(T, Prefix, [Processed | Result]).

-spec prefix(string(), string()) -> string().
prefix(Line, []) ->
  Line;
prefix(Line, Prefix) ->
  Prefixed = lists:prefix(Prefix, Line),
  if
    Prefixed ->
      string:substr(Line, length(Prefix) + 1);
    true ->
      Line
  end.

-spec common_indent([string()]) -> non_neg_integer().
common_indent(Lines) ->
  case common_indent(Lines, noindent) of
    noindent ->
      0;
    Indent ->
      Indent
  end.

-spec common_indent([string()], noindent | non_neg_integer()) -> noindent | non_neg_integer().
common_indent([], Indent) ->
    Indent;
common_indent([H | T], Indent) ->
  CurrentIndent = leading_whitespace(H),
  if
    (CurrentIndent < length(H)) and ((Indent == noindent) or (CurrentIndent < Indent)) ->
      common_indent(T, CurrentIndent);
    true ->
      common_indent(T, Indent)
  end.

-spec leading_whitespace(string()) -> non_neg_integer().
leading_whitespace(BlockStringValue) ->
  leading_whitespace(BlockStringValue, 0).

-spec leading_whitespace(string(), non_neg_integer()) -> non_neg_integer().
leading_whitespace([], N) ->
  N;
leading_whitespace([32 | T], N) ->
  leading_whitespace(T, N + 1);
leading_whitespace([$\t | T], N) ->
  leading_whitespace(T, N + 1);
leading_whitespace([_H | _T], N) ->
  N.

-spec is_blank(string()) -> boolean().
is_blank(BlockStringValue) ->
    leading_whitespace(BlockStringValue) == length(BlockStringValue).

% Integer

extract_integer({_Token, _Loc, Value}) ->
  {Int, []} = string:to_integer(Value), Int.

% Float

extract_float({_Token, _Loc, Value}) ->
  {Float, []} = string:to_float(Value), Float.

% Boolean

extract_boolean({_Token, _Loc, "true"}) ->
  true;
extract_boolean({_Token, _Loc, "false"}) ->
  false.

% Directive Placement

do_extract_directive_locations([], Converted) ->
  Converted;
do_extract_directive_locations([LocationBinary | Tail], Converted) ->
  LocationAtom = to_directive_location_atom(LocationBinary),
  do_extract_directive_locations(Tail, [LocationAtom | Converted]).

to_directive_location_atom(Name) ->
  erlang:binary_to_existing_atom(string:lowercase(Name), utf8).

extract_directive_locations(Locations) ->
  Result = do_extract_directive_locations(Locations, []),
  lists:sort(Result).

-file("/Users/will/.local/share/mise/installs/erlang/29.0.5/lib/parsetools-2.8/include/yeccpre.hrl", 0).
%%
%% %CopyrightBegin%
%%
%% SPDX-License-Identifier: Apache-2.0
%%
%% Copyright Ericsson AB 1996-2025. All Rights Reserved.
%%
%% Licensed under the Apache License, Version 2.0 (the "License");
%% you may not use this file except in compliance with the License.
%% You may obtain a copy of the License at
%%
%%     http://www.apache.org/licenses/LICENSE-2.0
%%
%% Unless required by applicable law or agreed to in writing, software
%% distributed under the License is distributed on an "AS IS" BASIS,
%% WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
%% See the License for the specific language governing permissions and
%% limitations under the License.
%%
%% %CopyrightEnd%
%%

%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%
% The parser generator will insert appropriate declarations before this line.%

-type yecc_ret() :: {'error', _} | {'ok', _}.

-ifdef (YECC_PARSE_DOC).
-doc ?YECC_PARSE_DOC.
-endif.
-spec parse(Tokens :: list()) -> yecc_ret().
parse(Tokens) ->
    yeccpars0(Tokens, {no_func, no_location}, 0, [], []).

-ifdef (YECC_PARSE_AND_SCAN_DOC).
-doc ?YECC_PARSE_AND_SCAN_DOC.
-endif.
-spec parse_and_scan({function() | {atom(), atom()}, [_]}
                     | {atom(), atom(), [_]}) -> yecc_ret().
parse_and_scan({F, A}) ->
    yeccpars0([], {{F, A}, no_location}, 0, [], []);
parse_and_scan({M, F, A}) ->
    Arity = length(A),
    yeccpars0([], {{fun M:F/Arity, A}, no_location}, 0, [], []).

-ifdef (YECC_FORMAT_ERROR_DOC).
-doc ?YECC_FORMAT_ERROR_DOC.
-endif.
-spec format_error(any()) -> [char() | list()].
format_error(Message) ->
    case io_lib:deep_char_list(Message) of
        true ->
            Message;
        _ ->
            io_lib:write(Message)
    end.

%% To be used in grammar files to throw an error message to the parser
%% toplevel. Doesn't have to be exported!
-compile({nowarn_unused_function, return_error/2}).
-spec return_error(erl_anno:location(), any()) -> no_return().
return_error(Location, Message) ->
    throw({error, {Location, ?MODULE, Message}}).

-define(CODE_VERSION, "1.4").

yeccpars0(Tokens, Tzr, State, States, Vstack) ->
    try yeccpars1(Tokens, Tzr, State, States, Vstack)
    catch 
        error: Error: Stacktrace ->
            try yecc_error_type(Error, Stacktrace) of
                Desc ->
                    erlang:raise(error, {yecc_bug, ?CODE_VERSION, Desc},
                                 Stacktrace)
            catch _:_ -> erlang:raise(error, Error, Stacktrace)
            end;
        %% Probably thrown from return_error/2:
        throw: {error, {_Location, ?MODULE, _M}} = Error ->
            Error
    end.

yecc_error_type(function_clause, [{?MODULE,F,ArityOrArgs,_} | _]) ->
    case atom_to_list(F) of
        "yeccgoto_" ++ SymbolL ->
            {ok,[{atom,_,Symbol}],_} = erl_scan:string(SymbolL),
            State = case ArityOrArgs of
                        [S,_,_,_,_,_,_] -> S;
                        _ -> state_is_unknown
                    end,
            {Symbol, State, missing_in_goto_table}
    end.

yeccpars1([Token | Tokens], Tzr, State, States, Vstack) ->
    yeccpars2(State, element(1, Token), States, Vstack, Token, Tokens, Tzr);
yeccpars1([], {{F, A},_Location}, State, States, Vstack) ->
    case apply(F, A) of
        {ok, Tokens, EndLocation} ->
            yeccpars1(Tokens, {{F, A}, EndLocation}, State, States, Vstack);
        {eof, EndLocation} ->
            yeccpars1([], {no_func, EndLocation}, State, States, Vstack);
        {error, Descriptor, _EndLocation} ->
            {error, Descriptor}
    end;
yeccpars1([], {no_func, no_location}, State, States, Vstack) ->
    Line = 999999,
    yeccpars2(State, '$end', States, Vstack, yecc_end(Line), [],
              {no_func, Line});
yeccpars1([], {no_func, EndLocation}, State, States, Vstack) ->
    yeccpars2(State, '$end', States, Vstack, yecc_end(EndLocation), [],
              {no_func, EndLocation}).

%% yeccpars1/7 is called from generated code.
%%
%% When using the {includefile, Includefile} option, make sure that
%% yeccpars1/7 can be found by parsing the file without following
%% include directives. yecc will otherwise assume that an old
%% yeccpre.hrl is included (one which defines yeccpars1/5).
yeccpars1(State1, State, States, Vstack, Token0, [Token | Tokens], Tzr) ->
    yeccpars2(State, element(1, Token), [State1 | States],
              [Token0 | Vstack], Token, Tokens, Tzr);
yeccpars1(State1, State, States, Vstack, Token0, [], {{_F,_A}, _Location}=Tzr) ->
    yeccpars1([], Tzr, State, [State1 | States], [Token0 | Vstack]);
yeccpars1(State1, State, States, Vstack, Token0, [], {no_func, no_location}) ->
    Location = yecctoken_end_location(Token0),
    yeccpars2(State, '$end', [State1 | States], [Token0 | Vstack],
              yecc_end(Location), [], {no_func, Location});
yeccpars1(State1, State, States, Vstack, Token0, [], {no_func, Location}) ->
    yeccpars2(State, '$end', [State1 | States], [Token0 | Vstack],
              yecc_end(Location), [], {no_func, Location}).

%% For internal use only.
yecc_end(Location) ->
    {'$end', Location}.

yecctoken_end_location(Token) ->
    try erl_anno:end_location(element(2, Token)) of
        undefined -> yecctoken_location(Token);
        Loc -> Loc
    catch _:_ -> yecctoken_location(Token)
    end.

-compile({nowarn_unused_function, yeccerror/1}).
yeccerror(Token) ->
    Text = yecctoken_to_string(Token),
    Location = yecctoken_location(Token),
    {error, {Location, ?MODULE, ["syntax error before: ", Text]}}.

-compile({nowarn_unused_function, yecctoken_to_string/1}).
yecctoken_to_string(Token) ->
    try erl_scan:text(Token) of
        undefined -> yecctoken2string(Token);
        Txt -> Txt
    catch _:_ -> yecctoken2string(Token)
    end.

yecctoken_location(Token) ->
    try erl_scan:location(Token)
    catch _:_ -> element(2, Token)
    end.

-compile({nowarn_unused_function, yecctoken2string/1}).
yecctoken2string(Token) ->
    try
        yecctoken2string1(Token)
    catch
        _:_ ->
            io_lib:format("~tp", [Token])
    end.

-compile({nowarn_unused_function, yecctoken2string1/1}).
yecctoken2string1({atom, _, A}) -> io_lib:write_atom(A);
yecctoken2string1({integer,_,N}) -> io_lib:write(N);
yecctoken2string1({float,_,F}) -> io_lib:write(F);
yecctoken2string1({char,_,C}) -> io_lib:write_char(C);
yecctoken2string1({var,_,V}) -> io_lib:format("~s", [V]);
yecctoken2string1({string,_,S}) -> io_lib:write_string(S);
yecctoken2string1({reserved_symbol, _, A}) -> io_lib:write(A);
yecctoken2string1({_Cat, _, Val}) -> io_lib:format("~tp", [Val]);
yecctoken2string1({dot, _}) -> "'.'";
yecctoken2string1({'$end', _}) -> [];
yecctoken2string1({Other, _}) when is_atom(Other) ->
    io_lib:write_atom(Other);
yecctoken2string1(Other) ->
    io_lib:format("~tp", [Other]).

%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%



-file("src/absinthe_parser.erl", 376).

-dialyzer({nowarn_function, yeccpars2/7}).
-compile({nowarn_unused_function,  yeccpars2/7}).
yeccpars2(0=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_0(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(1=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_1(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(2=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_2(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(3=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_3(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(4=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_4(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(5=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_5(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(6=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_6(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(7=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_7(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(8=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_8(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(9=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_9(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(10=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_10(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(11=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_11(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(12=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_12(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(13=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_13(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(14=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_14(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(15=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_15(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(16=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_16(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(17=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_17(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(18=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_18(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(19=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_19(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(20=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_20(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(21=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(22=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_22(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(23=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(24=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(25=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(26=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_26(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(27=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_27(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(28=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(29=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_29(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(30=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_30(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(31=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_31(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(32=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(33=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(34=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_34(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(35=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_35(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(36=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_36(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(37=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_37(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(38=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_38(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(39=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_39(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(40=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_40(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(41=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_41(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(42=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(43=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_43(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(44=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_44(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(45=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_45(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(46=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_46(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(47=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_47(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(48=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_48(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(49=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_49(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(50=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_50(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(51=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_51(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(52=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_52(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(53=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_53(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(54=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_54(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(55=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_55(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(56=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_56(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(57=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_57(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(58=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_58(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(59=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_59(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(60=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_60(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(61=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_61(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(62=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_62(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(63=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_63(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(64=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_64(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(65=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_65(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(66=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(67=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_67(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(68=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_68(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(69=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_69(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(70=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_70(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(71=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_63(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(72=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_72(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(73=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_73(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(74=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_74(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(75=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_75(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(76=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_76(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(77=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_77(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(78=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_78(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(79=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_79(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(80=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_80(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(81=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_81(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(82=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_82(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(83=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_83(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(84=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_84(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(85=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_85(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(86=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_86(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(87=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_87(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(88=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_88(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(89=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_89(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(90=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_90(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(91=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_91(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(92=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_92(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(93=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_93(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(94=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_94(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(95=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_95(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(96=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_96(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(97=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_97(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(98=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_98(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(99=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_99(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(100=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_100(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(101=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_81(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(102=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_102(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(103=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_103(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(104=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_104(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(105=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_105(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(106=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_106(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(107=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_107(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(108=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_108(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(109=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_109(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(110=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_110(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(111=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_111(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(112=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_112(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(113=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_113(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(114=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_81(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(115=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_115(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(116=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_116(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(117=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_117(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(118=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_118(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(119=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_119(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(120=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_120(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(121=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_121(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(122=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_122(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(123=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_123(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(124=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_124(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(125=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_125(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(126=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_126(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(127=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_127(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(128=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_128(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(129=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_129(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(130=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_130(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(131=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_131(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(132=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_132(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(133=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_133(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(134=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_134(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(135=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_135(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(136=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_136(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(137=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_137(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(138=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_138(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(139=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_139(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(140=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_140(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(141=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_141(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(142=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_142(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(143=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_143(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(144=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_144(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(145=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_145(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(146=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_146(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(147=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_147(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(148=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_148(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(149=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_149(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(150=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_150(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(151=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_151(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(152=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_152(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(153=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_153(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(154=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_154(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(155=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_155(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(156=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_156(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(157=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_157(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(158=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_158(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(159=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_159(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(160=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_160(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(161=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_161(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(162=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_162(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(163=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_163(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(164=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_164(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(165=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_165(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(166=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_166(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(167=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_167(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(168=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_151(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(169=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_169(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(170=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_170(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(171=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_171(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(172=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_172(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(173=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_173(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(174=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_174(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(175=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_175(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(176=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_176(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(177=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_177(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(178=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_178(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(179=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_151(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(180=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_180(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(181=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_181(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(182=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_182(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(183=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_183(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(184=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(185=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_185(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(186=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_141(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(187=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_187(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(188=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_141(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(189=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_189(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(190=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_190(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(191=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_141(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(192=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_192(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(193=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_193(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(194=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_194(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(195=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_195(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(196=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(197=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(198=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_198(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(199=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_199(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(200=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_200(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(201=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(202=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_202(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(203=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_203(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(204=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_204(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(205=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_205(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(206=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_206(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(207=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_207(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(208=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(209=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_209(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(210=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_210(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(211=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_211(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(212=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_212(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(213=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_213(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(214=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_209(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(215=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_215(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(216=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_216(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(217=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_217(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(218=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_218(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(219=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_219(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(220=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_220(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(221=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_221(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(222=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_222(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(223=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(224=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_224(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(225=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_225(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(226=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_226(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(227=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_227(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(228=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_209(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(229=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_229(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(230=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_230(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(231=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_231(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(232=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_151(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(233=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_233(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(234=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_234(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(235=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_209(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(236=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_236(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(237=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_237(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(238=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_209(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(239=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_239(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(240=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_240(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(241=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_241(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(242=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_242(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(243=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(244=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_244(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(245=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_245(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(246=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_246(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(247=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(248=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_248(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(249=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_249(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(250=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(251=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_251(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(252=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_252(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(253=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_253(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(254=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(255=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_255(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(256=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_256(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(257=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(258=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_258(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(259=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_259(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(260=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_260(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(261=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_261(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(262=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_262(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(263=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_263(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(264=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_264(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(265=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(266=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_266(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(267=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_267(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(268=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(269=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_269(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(270=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_270(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(271=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_271(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(272=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(273=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_273(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(274=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_274(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(275=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(276=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_276(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(277=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_277(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(278=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_278(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(279=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_279(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(280=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(281=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_281(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(282=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_282(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(283=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(284=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_284(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(285=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_285(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(286=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_286(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(287=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(288=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_67(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(289=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_289(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(290=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_63(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(291=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_291(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(292=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_292(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(293=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_293(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(294=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_294(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(295=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_295(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(296=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_296(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(297=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_297(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(298=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_298(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(299=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_299(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(300=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_300(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(301=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(302=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_302(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(303=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_303(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(304=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_304(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(305=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(306=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_306(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(307=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_307(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(308=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_308(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(309=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_309(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(310=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_310(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(311=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(312=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_312(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(313=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_313(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(314=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(315=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_315(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(316=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_316(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(317=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_317(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(318=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_318(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(319=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_317(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(320=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_320(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(321=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_321(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(322=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(323=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_323(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(324=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_317(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(325=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_325(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(326=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_326(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(327=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_317(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(328=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_328(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(329=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_329(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(330=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_330(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(331=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_317(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(332=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_332(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(333=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_317(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(334=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_334(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(335=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_335(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(336=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_336(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(337=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_337(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(338=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_338(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(339=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_339(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(340=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_340(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(341=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_341(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(342=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_342(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(343=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_343(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(344=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_344(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(345=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_345(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(346=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_346(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(347=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_347(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(348=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_348(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(349=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_67(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(350=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_350(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(351=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_351(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(352=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_63(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(353=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_353(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(354=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_354(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(355=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_355(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(356=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_356(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(357=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_209(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(358=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_358(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(359=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_359(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(360=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_360(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(361=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_361(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(362=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_362(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(363=S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_363(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(364=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_364(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(365=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_67(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(366=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_366(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(367=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_63(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(368=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_368(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(369=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_369(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(370=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_63(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(371=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_371(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(372=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_372(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(373=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_63(S, Cat, Ss, Stack, T, Ts, Tzr);
%% yeccpars2(374=S, Cat, Ss, Stack, T, Ts, Tzr) ->
%%  yeccpars2_374(S, Cat, Ss, Stack, T, Ts, Tzr);
yeccpars2(Other, _, _, _, _, _, _) ->
 erlang:error({yecc_bug,"1.4",{missing_state_in_action_table, Other}}).

yeccpars2_0(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 19, Ss, Stack, T, Ts, Tzr);
yeccpars2_0(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 20, Ss, Stack, T, Ts, Tzr);
yeccpars2_0(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 22, Ss, Stack, T, Ts, Tzr);
yeccpars2_0(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 23, Ss, Stack, T, Ts, Tzr);
yeccpars2_0(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 26, Ss, Stack, T, Ts, Tzr);
yeccpars2_0(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 27, Ss, Stack, T, Ts, Tzr);
yeccpars2_0(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 30, Ss, Stack, T, Ts, Tzr);
yeccpars2_0(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 31, Ss, Stack, T, Ts, Tzr);
yeccpars2_0(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_0(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_22(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_1/7}).
-compile({nowarn_unused_function,  yeccpars2_1/7}).
yeccpars2_1(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_1_(Stack),
 yeccgoto_TypeDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_2/7}).
-compile({nowarn_unused_function,  yeccpars2_2/7}).
yeccpars2_2(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_2_(Stack),
 yeccgoto_TypeDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_3/7}).
-compile({nowarn_unused_function,  yeccpars2_3/7}).
yeccpars2_3(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_3_(Stack),
 yeccgoto_Definition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_4/7}).
-compile({nowarn_unused_function,  yeccpars2_4/7}).
yeccpars2_4(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_4_(Stack),
 yeccgoto_OperationDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_5/7}).
-compile({nowarn_unused_function,  yeccpars2_5/7}).
yeccpars2_5(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_5_(Stack),
 yeccgoto_TypeDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_6/7}).
-compile({nowarn_unused_function,  yeccpars2_6/7}).
yeccpars2_6(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_6_(Stack),
 yeccgoto_TypeDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

yeccpars2_7(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 353, Ss, Stack, T, Ts, Tzr);
yeccpars2_7(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_7(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_7(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_7(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_8/7}).
-compile({nowarn_unused_function,  yeccpars2_8/7}).
yeccpars2_8(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_8_(Stack),
 yeccgoto_Definition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_9/7}).
-compile({nowarn_unused_function,  yeccpars2_9/7}).
yeccpars2_9(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_9_(Stack),
 yeccgoto_TypeDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_10/7}).
-compile({nowarn_unused_function,  yeccpars2_10/7}).
yeccpars2_10(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_10_(Stack),
 yeccgoto_TypeDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_11/7}).
-compile({nowarn_unused_function,  yeccpars2_11/7}).
yeccpars2_11(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_11_(Stack),
 yeccgoto_TypeDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_12/7}).
-compile({nowarn_unused_function,  yeccpars2_12/7}).
yeccpars2_12(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_12_(Stack),
 yeccgoto_Definition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_13/7}).
-compile({nowarn_unused_function,  yeccpars2_13/7}).
yeccpars2_13(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_13_(Stack),
 yeccgoto_TypeDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_14/7}).
-compile({nowarn_unused_function,  yeccpars2_14/7}).
yeccpars2_14(_S, '$end', _Ss, Stack, _T, _Ts, _Tzr) ->
 {ok, hd(Stack)};
yeccpars2_14(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_15/7}).
-compile({nowarn_unused_function,  yeccpars2_15/7}).
yeccpars2_15(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_15_(Stack),
 yeccgoto_TypeDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

yeccpars2_16(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 20, Ss, Stack, T, Ts, Tzr);
yeccpars2_16(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 23, Ss, Stack, T, Ts, Tzr);
yeccpars2_16(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 26, Ss, Stack, T, Ts, Tzr);
yeccpars2_16(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 27, Ss, Stack, T, Ts, Tzr);
yeccpars2_16(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 31, Ss, Stack, T, Ts, Tzr);
yeccpars2_16(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_16(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_22(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_17/7}).
-compile({nowarn_unused_function,  yeccpars2_17/7}).
yeccpars2_17(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_17_(Stack),
 yeccgoto_Document(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_18/7}).
-compile({nowarn_unused_function,  yeccpars2_18/7}).
yeccpars2_18(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 19, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 20, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 21, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 22, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 23, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 24, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 25, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 26, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 27, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 28, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 29, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 30, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 31, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 32, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 33, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_18(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_18_(Stack),
 yeccgoto_Definitions(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_19/7}).
-compile({nowarn_unused_function,  yeccpars2_19/7}).
yeccpars2_19(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_19_(Stack),
 yeccgoto_DescriptionDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_20/7}).
-compile({nowarn_unused_function,  yeccpars2_20/7}).
yeccpars2_20(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 314, Ss, Stack, T, Ts, Tzr);
yeccpars2_20(_, _, _, _, T, _, _) ->
 yeccerror(T).

yeccpars2_21(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_21(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_22/7}).
-compile({nowarn_unused_function,  yeccpars2_22/7}).
yeccpars2_22(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 21, Ss, Stack, T, Ts, Tzr);
yeccpars2_22(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 24, Ss, Stack, T, Ts, Tzr);
yeccpars2_22(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 25, Ss, Stack, T, Ts, Tzr);
yeccpars2_22(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 28, Ss, Stack, T, Ts, Tzr);
yeccpars2_22(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 29, Ss, Stack, T, Ts, Tzr);
yeccpars2_22(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 32, Ss, Stack, T, Ts, Tzr);
yeccpars2_22(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 33, Ss, Stack, T, Ts, Tzr);
yeccpars2_22(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_23/7}).
-compile({nowarn_unused_function,  yeccpars2_23/7}).
yeccpars2_23(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_23(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_24: see yeccpars2_21

%% yeccpars2_25: see yeccpars2_21

-dialyzer({nowarn_function, yeccpars2_26/7}).
-compile({nowarn_unused_function,  yeccpars2_26/7}).
yeccpars2_26(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_26_(Stack),
 yeccgoto_OperationType(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_27/7}).
-compile({nowarn_unused_function,  yeccpars2_27/7}).
yeccpars2_27(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_27_(Stack),
 yeccgoto_OperationType(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

%% yeccpars2_28: see yeccpars2_21

-dialyzer({nowarn_function, yeccpars2_29/7}).
-compile({nowarn_unused_function,  yeccpars2_29/7}).
yeccpars2_29(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_29(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 254, Ss, Stack, T, Ts, Tzr);
yeccpars2_29(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_29_(Stack),
 yeccgoto_SchemaDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_30/7}).
-compile({nowarn_unused_function,  yeccpars2_30/7}).
yeccpars2_30(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_30_(Stack),
 yeccgoto_DescriptionDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_31/7}).
-compile({nowarn_unused_function,  yeccpars2_31/7}).
yeccpars2_31(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_31_(Stack),
 yeccgoto_OperationType(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

%% yeccpars2_32: see yeccpars2_21

%% yeccpars2_33: see yeccpars2_21

yeccpars2_34(S, '...', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 43, Ss, Stack, T, Ts, Tzr);
yeccpars2_34(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_34(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_35/7}).
-compile({nowarn_unused_function,  yeccpars2_35/7}).
yeccpars2_35(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 137, Ss, Stack, T, Ts, Tzr);
yeccpars2_35(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_36/7}).
-compile({nowarn_unused_function,  yeccpars2_36/7}).
yeccpars2_36(S, '...', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 43, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_36(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_36_(Stack),
 yeccgoto_Selections(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_37/7}).
-compile({nowarn_unused_function,  yeccpars2_37/7}).
yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_37_(Stack),
 yeccgoto_Name(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_38/7}).
-compile({nowarn_unused_function,  yeccpars2_38/7}).
yeccpars2_38(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 76, Ss, Stack, T, Ts, Tzr);
yeccpars2_38(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 131, Ss, Stack, T, Ts, Tzr);
yeccpars2_38(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_38(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_38(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_38_(Stack),
 yeccgoto_Field(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_39/7}).
-compile({nowarn_unused_function,  yeccpars2_39/7}).
yeccpars2_39(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_39_(Stack),
 yeccgoto_Selection(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_40/7}).
-compile({nowarn_unused_function,  yeccpars2_40/7}).
yeccpars2_40(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_40_(Stack),
 yeccgoto_Selection(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_41/7}).
-compile({nowarn_unused_function,  yeccpars2_41/7}).
yeccpars2_41(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_41_(Stack),
 yeccgoto_Selection(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

%% yeccpars2_42: see yeccpars2_21

yeccpars2_43(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_43(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 66, Ss, Stack, T, Ts, Tzr);
yeccpars2_43(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_43(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_44/7}).
-compile({nowarn_unused_function,  yeccpars2_44/7}).
yeccpars2_44(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_44_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_45/7}).
-compile({nowarn_unused_function,  yeccpars2_45/7}).
yeccpars2_45(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_45_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_46/7}).
-compile({nowarn_unused_function,  yeccpars2_46/7}).
yeccpars2_46(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_46_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_47/7}).
-compile({nowarn_unused_function,  yeccpars2_47/7}).
yeccpars2_47(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_47_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_48/7}).
-compile({nowarn_unused_function,  yeccpars2_48/7}).
yeccpars2_48(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_48_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_49/7}).
-compile({nowarn_unused_function,  yeccpars2_49/7}).
yeccpars2_49(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_49_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_50/7}).
-compile({nowarn_unused_function,  yeccpars2_50/7}).
yeccpars2_50(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_50_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_51/7}).
-compile({nowarn_unused_function,  yeccpars2_51/7}).
yeccpars2_51(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_51_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_52/7}).
-compile({nowarn_unused_function,  yeccpars2_52/7}).
yeccpars2_52(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_52_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_53/7}).
-compile({nowarn_unused_function,  yeccpars2_53/7}).
yeccpars2_53(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_53_(Stack),
 yeccgoto_Name(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_54/7}).
-compile({nowarn_unused_function,  yeccpars2_54/7}).
yeccpars2_54(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_54_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_55/7}).
-compile({nowarn_unused_function,  yeccpars2_55/7}).
yeccpars2_55(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_55_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_56/7}).
-compile({nowarn_unused_function,  yeccpars2_56/7}).
yeccpars2_56(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_56_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_57/7}).
-compile({nowarn_unused_function,  yeccpars2_57/7}).
yeccpars2_57(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_57_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_58/7}).
-compile({nowarn_unused_function,  yeccpars2_58/7}).
yeccpars2_58(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_58_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_59/7}).
-compile({nowarn_unused_function,  yeccpars2_59/7}).
yeccpars2_59(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_59_(Stack),
 yeccgoto_NameWithoutOn(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_60/7}).
-compile({nowarn_unused_function,  yeccpars2_60/7}).
yeccpars2_60(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_60_(Stack),
 yeccgoto_InlineFragment(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_61/7}).
-compile({nowarn_unused_function,  yeccpars2_61/7}).
yeccpars2_61(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_61_(Stack),
 yeccgoto_FragmentName(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_62/7}).
-compile({nowarn_unused_function,  yeccpars2_62/7}).
yeccpars2_62(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_62(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_62_(Stack),
 yeccgoto_FragmentSpread(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_63/7}).
-compile({nowarn_unused_function,  yeccpars2_63/7}).
yeccpars2_63(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_63(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_64/7}).
-compile({nowarn_unused_function,  yeccpars2_64/7}).
yeccpars2_64(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_64(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_64_(Stack),
 yeccgoto_Directives(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

yeccpars2_65(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 74, Ss, Stack, T, Ts, Tzr);
yeccpars2_65(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

%% yeccpars2_66: see yeccpars2_21

-dialyzer({nowarn_function, yeccpars2_67/7}).
-compile({nowarn_unused_function,  yeccpars2_67/7}).
yeccpars2_67(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_67(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_67(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_68/7}).
-compile({nowarn_unused_function,  yeccpars2_68/7}).
yeccpars2_68(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_68_(Stack),
 yeccgoto_TypeCondition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_69/7}).
-compile({nowarn_unused_function,  yeccpars2_69/7}).
yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_69_(Stack),
 yeccgoto_NamedType(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_70/7}).
-compile({nowarn_unused_function,  yeccpars2_70/7}).
yeccpars2_70(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_70_(Stack),
 yeccgoto_InlineFragment(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_71: see yeccpars2_63

-dialyzer({nowarn_function, yeccpars2_72/7}).
-compile({nowarn_unused_function,  yeccpars2_72/7}).
yeccpars2_72(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_72_(Stack),
 yeccgoto_InlineFragment(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_73/7}).
-compile({nowarn_unused_function,  yeccpars2_73/7}).
yeccpars2_73(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 76, Ss, Stack, T, Ts, Tzr);
yeccpars2_73(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_73_(Stack),
 yeccgoto_Directive(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_74/7}).
-compile({nowarn_unused_function,  yeccpars2_74/7}).
yeccpars2_74(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 76, Ss, Stack, T, Ts, Tzr);
yeccpars2_74(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_74_(Stack),
 yeccgoto_Directive(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_75/7}).
-compile({nowarn_unused_function,  yeccpars2_75/7}).
yeccpars2_75(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_75_(Stack),
 yeccgoto_Directive(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

yeccpars2_76(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 80, Ss, Stack, T, Ts, Tzr);
yeccpars2_76(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_77/7}).
-compile({nowarn_unused_function,  yeccpars2_77/7}).
yeccpars2_77(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 114, Ss, Stack, T, Ts, Tzr);
yeccpars2_77(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_78/7}).
-compile({nowarn_unused_function,  yeccpars2_78/7}).
yeccpars2_78(S, ')', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 113, Ss, Stack, T, Ts, Tzr);
yeccpars2_78(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_79/7}).
-compile({nowarn_unused_function,  yeccpars2_79/7}).
yeccpars2_79(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 80, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_79(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_79_(Stack),
 yeccgoto_ArgumentList(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_80/7}).
-compile({nowarn_unused_function,  yeccpars2_80/7}).
yeccpars2_80(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 81, Ss, Stack, T, Ts, Tzr);
yeccpars2_80(_, _, _, _, T, _, _) ->
 yeccerror(T).

yeccpars2_81(S, '$', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 88, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, '[', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 89, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 90, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, 'boolean_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 91, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, 'float_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 92, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, 'int_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 93, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, 'null', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 94, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 95, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 96, Ss, Stack, T, Ts, Tzr);
yeccpars2_81(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_82/7}).
-compile({nowarn_unused_function,  yeccpars2_82/7}).
yeccpars2_82(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_82_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_83/7}).
-compile({nowarn_unused_function,  yeccpars2_83/7}).
yeccpars2_83(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_83_(Stack),
 yeccgoto_Argument(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_84/7}).
-compile({nowarn_unused_function,  yeccpars2_84/7}).
yeccpars2_84(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_84_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_85/7}).
-compile({nowarn_unused_function,  yeccpars2_85/7}).
yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_85_(Stack),
 yeccgoto_EnumValue(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_86/7}).
-compile({nowarn_unused_function,  yeccpars2_86/7}).
yeccpars2_86(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_86_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_87/7}).
-compile({nowarn_unused_function,  yeccpars2_87/7}).
yeccpars2_87(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_87_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

yeccpars2_88(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 111, Ss, Stack, T, Ts, Tzr);
yeccpars2_88(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

yeccpars2_89(S, '$', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 88, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, '[', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 89, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, ']', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 107, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 90, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, 'boolean_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 91, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, 'float_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 92, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, 'int_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 93, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, 'null', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 94, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 95, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 96, Ss, Stack, T, Ts, Tzr);
yeccpars2_89(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_90/7}).
-compile({nowarn_unused_function,  yeccpars2_90/7}).
yeccpars2_90(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_90_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_91/7}).
-compile({nowarn_unused_function,  yeccpars2_91/7}).
yeccpars2_91(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_91_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_92/7}).
-compile({nowarn_unused_function,  yeccpars2_92/7}).
yeccpars2_92(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_92_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_93/7}).
-compile({nowarn_unused_function,  yeccpars2_93/7}).
yeccpars2_93(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_93_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_94/7}).
-compile({nowarn_unused_function,  yeccpars2_94/7}).
yeccpars2_94(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_94_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_95/7}).
-compile({nowarn_unused_function,  yeccpars2_95/7}).
yeccpars2_95(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_95_(Stack),
 yeccgoto_Value(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

yeccpars2_96(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_96(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 100, Ss, Stack, T, Ts, Tzr);
yeccpars2_96(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_97/7}).
-compile({nowarn_unused_function,  yeccpars2_97/7}).
yeccpars2_97(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 104, Ss, Stack, T, Ts, Tzr);
yeccpars2_97(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_98/7}).
-compile({nowarn_unused_function,  yeccpars2_98/7}).
yeccpars2_98(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_98(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_98_(Stack),
 yeccgoto_ObjectFields(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_99/7}).
-compile({nowarn_unused_function,  yeccpars2_99/7}).
yeccpars2_99(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 101, Ss, Stack, T, Ts, Tzr);
yeccpars2_99(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_100/7}).
-compile({nowarn_unused_function,  yeccpars2_100/7}).
yeccpars2_100(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_100_(Stack),
 yeccgoto_ObjectValue(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_101: see yeccpars2_81

-dialyzer({nowarn_function, yeccpars2_102/7}).
-compile({nowarn_unused_function,  yeccpars2_102/7}).
yeccpars2_102(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_102_(Stack),
 yeccgoto_ObjectField(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_103/7}).
-compile({nowarn_unused_function,  yeccpars2_103/7}).
yeccpars2_103(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_103_(Stack),
 yeccgoto_ObjectFields(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_104/7}).
-compile({nowarn_unused_function,  yeccpars2_104/7}).
yeccpars2_104(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_104_(Stack),
 yeccgoto_ObjectValue(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_105/7}).
-compile({nowarn_unused_function,  yeccpars2_105/7}).
yeccpars2_105(S, ']', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 109, Ss, Stack, T, Ts, Tzr);
yeccpars2_105(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_106/7}).
-compile({nowarn_unused_function,  yeccpars2_106/7}).
yeccpars2_106(S, '$', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 88, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, '[', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 89, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 90, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'boolean_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 91, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'float_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 92, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'int_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 93, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'null', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 94, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 95, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 96, Ss, Stack, T, Ts, Tzr);
yeccpars2_106(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_106_(Stack),
 yeccgoto_Values(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_107/7}).
-compile({nowarn_unused_function,  yeccpars2_107/7}).
yeccpars2_107(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_107_(Stack),
 yeccgoto_ListValue(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_108/7}).
-compile({nowarn_unused_function,  yeccpars2_108/7}).
yeccpars2_108(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_108_(Stack),
 yeccgoto_Values(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_109/7}).
-compile({nowarn_unused_function,  yeccpars2_109/7}).
yeccpars2_109(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_109_(Stack),
 yeccgoto_ListValue(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_110/7}).
-compile({nowarn_unused_function,  yeccpars2_110/7}).
yeccpars2_110(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_110_(Stack),
 yeccgoto_Variable(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_111/7}).
-compile({nowarn_unused_function,  yeccpars2_111/7}).
yeccpars2_111(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_111_(Stack),
 yeccgoto_Variable(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_112/7}).
-compile({nowarn_unused_function,  yeccpars2_112/7}).
yeccpars2_112(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_112_(Stack),
 yeccgoto_ArgumentList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_113/7}).
-compile({nowarn_unused_function,  yeccpars2_113/7}).
yeccpars2_113(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_113_(Stack),
 yeccgoto_Arguments(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_114: see yeccpars2_81

-dialyzer({nowarn_function, yeccpars2_115/7}).
-compile({nowarn_unused_function,  yeccpars2_115/7}).
yeccpars2_115(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_115_(Stack),
 yeccgoto_Argument(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_116/7}).
-compile({nowarn_unused_function,  yeccpars2_116/7}).
yeccpars2_116(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_116_(Stack),
 yeccgoto_Directive(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_117/7}).
-compile({nowarn_unused_function,  yeccpars2_117/7}).
yeccpars2_117(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_117_(Stack),
 yeccgoto_Directives(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_118/7}).
-compile({nowarn_unused_function,  yeccpars2_118/7}).
yeccpars2_118(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_118_(Stack),
 yeccgoto_InlineFragment(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_119/7}).
-compile({nowarn_unused_function,  yeccpars2_119/7}).
yeccpars2_119(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_119_(Stack),
 yeccgoto_FragmentSpread(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_120/7}).
-compile({nowarn_unused_function,  yeccpars2_120/7}).
yeccpars2_120(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 76, Ss, Stack, T, Ts, Tzr);
yeccpars2_120(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_120(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_120(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_120_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_121/7}).
-compile({nowarn_unused_function,  yeccpars2_121/7}).
yeccpars2_121(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_121_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_122/7}).
-compile({nowarn_unused_function,  yeccpars2_122/7}).
yeccpars2_122(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_122(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_122_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_123/7}).
-compile({nowarn_unused_function,  yeccpars2_123/7}).
yeccpars2_123(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_123(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_123(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_123_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_124/7}).
-compile({nowarn_unused_function,  yeccpars2_124/7}).
yeccpars2_124(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_124_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_125/7}).
-compile({nowarn_unused_function,  yeccpars2_125/7}).
yeccpars2_125(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_125(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_125_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_126/7}).
-compile({nowarn_unused_function,  yeccpars2_126/7}).
yeccpars2_126(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_126_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_127/7}).
-compile({nowarn_unused_function,  yeccpars2_127/7}).
yeccpars2_127(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_127_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_128/7}).
-compile({nowarn_unused_function,  yeccpars2_128/7}).
yeccpars2_128(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_128_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_129/7}).
-compile({nowarn_unused_function,  yeccpars2_129/7}).
yeccpars2_129(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_129(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_129_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_130/7}).
-compile({nowarn_unused_function,  yeccpars2_130/7}).
yeccpars2_130(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_130(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_130(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_130_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_131/7}).
-compile({nowarn_unused_function,  yeccpars2_131/7}).
yeccpars2_131(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_131_(Stack),
 yeccgoto_Alias(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_132/7}).
-compile({nowarn_unused_function,  yeccpars2_132/7}).
yeccpars2_132(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_132_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_133/7}).
-compile({nowarn_unused_function,  yeccpars2_133/7}).
yeccpars2_133(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_133(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_133_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_134/7}).
-compile({nowarn_unused_function,  yeccpars2_134/7}).
yeccpars2_134(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_134_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_135/7}).
-compile({nowarn_unused_function,  yeccpars2_135/7}).
yeccpars2_135(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_135_(Stack),
 yeccgoto_Field(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_136/7}).
-compile({nowarn_unused_function,  yeccpars2_136/7}).
yeccpars2_136(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_136_(Stack),
 yeccgoto_Selections(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_137/7}).
-compile({nowarn_unused_function,  yeccpars2_137/7}).
yeccpars2_137(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_137_(Stack),
 yeccgoto_SelectionSet(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_138/7}).
-compile({nowarn_unused_function,  yeccpars2_138/7}).
yeccpars2_138(S, '=', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 141, Ss, Stack, T, Ts, Tzr);
yeccpars2_138(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_138(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_138_(Stack),
 yeccgoto_UnionTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_139/7}).
-compile({nowarn_unused_function,  yeccpars2_139/7}).
yeccpars2_139(S, '=', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 191, Ss, Stack, T, Ts, Tzr);
yeccpars2_139(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_139_(Stack),
 yeccgoto_UnionTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_140/7}).
-compile({nowarn_unused_function,  yeccpars2_140/7}).
yeccpars2_140(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_140(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_140_(Stack),
 yeccgoto_DirectivesConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

yeccpars2_141(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_141(S, '|', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 184, Ss, Stack, T, Ts, Tzr);
yeccpars2_141(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

yeccpars2_142(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 144, Ss, Stack, T, Ts, Tzr);
yeccpars2_142(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_143/7}).
-compile({nowarn_unused_function,  yeccpars2_143/7}).
yeccpars2_143(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 146, Ss, Stack, T, Ts, Tzr);
yeccpars2_143(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_143_(Stack),
 yeccgoto_DirectiveConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_144/7}).
-compile({nowarn_unused_function,  yeccpars2_144/7}).
yeccpars2_144(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 146, Ss, Stack, T, Ts, Tzr);
yeccpars2_144(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_144_(Stack),
 yeccgoto_DirectiveConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_145/7}).
-compile({nowarn_unused_function,  yeccpars2_145/7}).
yeccpars2_145(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_145_(Stack),
 yeccgoto_DirectiveConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

yeccpars2_146(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 150, Ss, Stack, T, Ts, Tzr);
yeccpars2_146(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_147/7}).
-compile({nowarn_unused_function,  yeccpars2_147/7}).
yeccpars2_147(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 179, Ss, Stack, T, Ts, Tzr);
yeccpars2_147(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_148/7}).
-compile({nowarn_unused_function,  yeccpars2_148/7}).
yeccpars2_148(S, ')', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 178, Ss, Stack, T, Ts, Tzr);
yeccpars2_148(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_149/7}).
-compile({nowarn_unused_function,  yeccpars2_149/7}).
yeccpars2_149(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 150, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_149(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_149_(Stack),
 yeccgoto_ArgumentListConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_150/7}).
-compile({nowarn_unused_function,  yeccpars2_150/7}).
yeccpars2_150(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 151, Ss, Stack, T, Ts, Tzr);
yeccpars2_150(_, _, _, _, T, _, _) ->
 yeccerror(T).

yeccpars2_151(S, '[', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 156, Ss, Stack, T, Ts, Tzr);
yeccpars2_151(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 157, Ss, Stack, T, Ts, Tzr);
yeccpars2_151(S, 'boolean_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 158, Ss, Stack, T, Ts, Tzr);
yeccpars2_151(S, 'float_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 159, Ss, Stack, T, Ts, Tzr);
yeccpars2_151(S, 'int_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 160, Ss, Stack, T, Ts, Tzr);
yeccpars2_151(S, 'null', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 161, Ss, Stack, T, Ts, Tzr);
yeccpars2_151(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_151(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 162, Ss, Stack, T, Ts, Tzr);
yeccpars2_151(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 163, Ss, Stack, T, Ts, Tzr);
yeccpars2_151(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_152/7}).
-compile({nowarn_unused_function,  yeccpars2_152/7}).
yeccpars2_152(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_152_(Stack),
 yeccgoto_ArgumentConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_153/7}).
-compile({nowarn_unused_function,  yeccpars2_153/7}).
yeccpars2_153(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_153_(Stack),
 yeccgoto_ValueConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_154/7}).
-compile({nowarn_unused_function,  yeccpars2_154/7}).
yeccpars2_154(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_154_(Stack),
 yeccgoto_ValueConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_155/7}).
-compile({nowarn_unused_function,  yeccpars2_155/7}).
yeccpars2_155(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_155_(Stack),
 yeccgoto_ValueConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

yeccpars2_156(S, '[', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 156, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, ']', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 174, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 157, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, 'boolean_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 158, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, 'float_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 159, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, 'int_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 160, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, 'null', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 161, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 162, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 163, Ss, Stack, T, Ts, Tzr);
yeccpars2_156(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_157/7}).
-compile({nowarn_unused_function,  yeccpars2_157/7}).
yeccpars2_157(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_157_(Stack),
 yeccgoto_ValueConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_158/7}).
-compile({nowarn_unused_function,  yeccpars2_158/7}).
yeccpars2_158(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_158_(Stack),
 yeccgoto_ValueConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_159/7}).
-compile({nowarn_unused_function,  yeccpars2_159/7}).
yeccpars2_159(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_159_(Stack),
 yeccgoto_ValueConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_160/7}).
-compile({nowarn_unused_function,  yeccpars2_160/7}).
yeccpars2_160(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_160_(Stack),
 yeccgoto_ValueConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_161/7}).
-compile({nowarn_unused_function,  yeccpars2_161/7}).
yeccpars2_161(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_161_(Stack),
 yeccgoto_ValueConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_162/7}).
-compile({nowarn_unused_function,  yeccpars2_162/7}).
yeccpars2_162(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_162_(Stack),
 yeccgoto_ValueConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

yeccpars2_163(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_163(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 167, Ss, Stack, T, Ts, Tzr);
yeccpars2_163(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_164/7}).
-compile({nowarn_unused_function,  yeccpars2_164/7}).
yeccpars2_164(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 171, Ss, Stack, T, Ts, Tzr);
yeccpars2_164(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_165/7}).
-compile({nowarn_unused_function,  yeccpars2_165/7}).
yeccpars2_165(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_165(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_165_(Stack),
 yeccgoto_ObjectFieldsConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_166/7}).
-compile({nowarn_unused_function,  yeccpars2_166/7}).
yeccpars2_166(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 168, Ss, Stack, T, Ts, Tzr);
yeccpars2_166(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_167/7}).
-compile({nowarn_unused_function,  yeccpars2_167/7}).
yeccpars2_167(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_167_(Stack),
 yeccgoto_ObjectValueConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_168: see yeccpars2_151

-dialyzer({nowarn_function, yeccpars2_169/7}).
-compile({nowarn_unused_function,  yeccpars2_169/7}).
yeccpars2_169(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_169_(Stack),
 yeccgoto_ObjectFieldConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_170/7}).
-compile({nowarn_unused_function,  yeccpars2_170/7}).
yeccpars2_170(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_170_(Stack),
 yeccgoto_ObjectFieldsConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_171/7}).
-compile({nowarn_unused_function,  yeccpars2_171/7}).
yeccpars2_171(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_171_(Stack),
 yeccgoto_ObjectValueConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_172/7}).
-compile({nowarn_unused_function,  yeccpars2_172/7}).
yeccpars2_172(S, ']', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 176, Ss, Stack, T, Ts, Tzr);
yeccpars2_172(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_173/7}).
-compile({nowarn_unused_function,  yeccpars2_173/7}).
yeccpars2_173(S, '[', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 156, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 157, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'boolean_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 158, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'float_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 159, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'int_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 160, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'null', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 161, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 162, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 163, Ss, Stack, T, Ts, Tzr);
yeccpars2_173(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_173_(Stack),
 yeccgoto_ValuesConst(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_174/7}).
-compile({nowarn_unused_function,  yeccpars2_174/7}).
yeccpars2_174(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_174_(Stack),
 yeccgoto_ListValueConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_175/7}).
-compile({nowarn_unused_function,  yeccpars2_175/7}).
yeccpars2_175(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_175_(Stack),
 yeccgoto_ValuesConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_176/7}).
-compile({nowarn_unused_function,  yeccpars2_176/7}).
yeccpars2_176(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_176_(Stack),
 yeccgoto_ListValueConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_177/7}).
-compile({nowarn_unused_function,  yeccpars2_177/7}).
yeccpars2_177(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_177_(Stack),
 yeccgoto_ArgumentListConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_178/7}).
-compile({nowarn_unused_function,  yeccpars2_178/7}).
yeccpars2_178(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_178_(Stack),
 yeccgoto_ArgumentsConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_179: see yeccpars2_151

-dialyzer({nowarn_function, yeccpars2_180/7}).
-compile({nowarn_unused_function,  yeccpars2_180/7}).
yeccpars2_180(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_180_(Stack),
 yeccgoto_ArgumentConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_181/7}).
-compile({nowarn_unused_function,  yeccpars2_181/7}).
yeccpars2_181(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_181_(Stack),
 yeccgoto_DirectiveConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_182/7}).
-compile({nowarn_unused_function,  yeccpars2_182/7}).
yeccpars2_182(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_182_(Stack),
 yeccgoto_UnionTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_183/7}).
-compile({nowarn_unused_function,  yeccpars2_183/7}).
yeccpars2_183(S, '|', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 188, Ss, Stack, T, Ts, Tzr);
yeccpars2_183(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_183_(Stack),
 yeccgoto_UnionMembers(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

%% yeccpars2_184: see yeccpars2_21

-dialyzer({nowarn_function, yeccpars2_185/7}).
-compile({nowarn_unused_function,  yeccpars2_185/7}).
yeccpars2_185(S, '|', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 186, Ss, Stack, T, Ts, Tzr);
yeccpars2_185(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_186: see yeccpars2_141

-dialyzer({nowarn_function, yeccpars2_187/7}).
-compile({nowarn_unused_function,  yeccpars2_187/7}).
yeccpars2_187(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_187_(Stack),
 yeccgoto_UnionMembers(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_188: see yeccpars2_141

-dialyzer({nowarn_function, yeccpars2_189/7}).
-compile({nowarn_unused_function,  yeccpars2_189/7}).
yeccpars2_189(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_189_(Stack),
 yeccgoto_UnionMembers(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_190/7}).
-compile({nowarn_unused_function,  yeccpars2_190/7}).
yeccpars2_190(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_190_(Stack),
 yeccgoto_DirectivesConst(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_191: see yeccpars2_141

-dialyzer({nowarn_function, yeccpars2_192/7}).
-compile({nowarn_unused_function,  yeccpars2_192/7}).
yeccpars2_192(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_192_(Stack),
 yeccgoto_UnionTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_193/7}).
-compile({nowarn_unused_function,  yeccpars2_193/7}).
yeccpars2_193(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_193(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 196, Ss, Stack, T, Ts, Tzr);
yeccpars2_193(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 197, Ss, Stack, T, Ts, Tzr);
yeccpars2_193(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_193_(Stack),
 yeccgoto_ObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_194/7}).
-compile({nowarn_unused_function,  yeccpars2_194/7}).
yeccpars2_194(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_194(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 247, Ss, Stack, T, Ts, Tzr);
yeccpars2_194(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_195/7}).
-compile({nowarn_unused_function,  yeccpars2_195/7}).
yeccpars2_195(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 243, Ss, Stack, T, Ts, Tzr);
yeccpars2_195(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_195_(Stack),
 yeccgoto_ObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_196: see yeccpars2_21

yeccpars2_197(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 19, Ss, Stack, T, Ts, Tzr);
yeccpars2_197(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_197(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 30, Ss, Stack, T, Ts, Tzr);
yeccpars2_197(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_198/7}).
-compile({nowarn_unused_function,  yeccpars2_198/7}).
yeccpars2_198(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 208, Ss, Stack, T, Ts, Tzr);
yeccpars2_198(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 209, Ss, Stack, T, Ts, Tzr);
yeccpars2_198(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_198(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_199/7}).
-compile({nowarn_unused_function,  yeccpars2_199/7}).
yeccpars2_199(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 205, Ss, Stack, T, Ts, Tzr);
yeccpars2_199(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_200/7}).
-compile({nowarn_unused_function,  yeccpars2_200/7}).
yeccpars2_200(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 19, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 30, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_200(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_200_(Stack),
 yeccgoto_FieldDefinitionList(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

%% yeccpars2_201: see yeccpars2_21

-dialyzer({nowarn_function, yeccpars2_202/7}).
-compile({nowarn_unused_function,  yeccpars2_202/7}).
yeccpars2_202(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 19, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 30, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_202(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_202_(Stack),
 yeccgoto_FieldDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_203/7}).
-compile({nowarn_unused_function,  yeccpars2_203/7}).
yeccpars2_203(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_203_(Stack),
 yeccgoto_FieldDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_204/7}).
-compile({nowarn_unused_function,  yeccpars2_204/7}).
yeccpars2_204(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_204_(Stack),
 yeccgoto_FieldDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_205/7}).
-compile({nowarn_unused_function,  yeccpars2_205/7}).
yeccpars2_205(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_205_(Stack),
 yeccgoto_ObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_206/7}).
-compile({nowarn_unused_function,  yeccpars2_206/7}).
yeccpars2_206(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 238, Ss, Stack, T, Ts, Tzr);
yeccpars2_206(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_207/7}).
-compile({nowarn_unused_function,  yeccpars2_207/7}).
yeccpars2_207(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 235, Ss, Stack, T, Ts, Tzr);
yeccpars2_207(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_208: see yeccpars2_197

yeccpars2_209(S, '[', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 214, Ss, Stack, T, Ts, Tzr);
yeccpars2_209(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_209(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_210/7}).
-compile({nowarn_unused_function,  yeccpars2_210/7}).
yeccpars2_210(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_210(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_210_(Stack),
 yeccgoto_FieldDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_211/7}).
-compile({nowarn_unused_function,  yeccpars2_211/7}).
yeccpars2_211(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_211_(Stack),
 yeccgoto_Type(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_212/7}).
-compile({nowarn_unused_function,  yeccpars2_212/7}).
yeccpars2_212(S, '!', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 218, Ss, Stack, T, Ts, Tzr);
yeccpars2_212(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_212_(Stack),
 yeccgoto_Type(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_213/7}).
-compile({nowarn_unused_function,  yeccpars2_213/7}).
yeccpars2_213(S, '!', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 217, Ss, Stack, T, Ts, Tzr);
yeccpars2_213(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_213_(Stack),
 yeccgoto_Type(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

%% yeccpars2_214: see yeccpars2_209

-dialyzer({nowarn_function, yeccpars2_215/7}).
-compile({nowarn_unused_function,  yeccpars2_215/7}).
yeccpars2_215(S, ']', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 216, Ss, Stack, T, Ts, Tzr);
yeccpars2_215(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_216/7}).
-compile({nowarn_unused_function,  yeccpars2_216/7}).
yeccpars2_216(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_216_(Stack),
 yeccgoto_ListType(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_217/7}).
-compile({nowarn_unused_function,  yeccpars2_217/7}).
yeccpars2_217(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_217_(Stack),
 yeccgoto_NonNullType(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_218/7}).
-compile({nowarn_unused_function,  yeccpars2_218/7}).
yeccpars2_218(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_218_(Stack),
 yeccgoto_NonNullType(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_219/7}).
-compile({nowarn_unused_function,  yeccpars2_219/7}).
yeccpars2_219(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_219_(Stack),
 yeccgoto_FieldDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_220/7}).
-compile({nowarn_unused_function,  yeccpars2_220/7}).
yeccpars2_220(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 228, Ss, Stack, T, Ts, Tzr);
yeccpars2_220(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_221/7}).
-compile({nowarn_unused_function,  yeccpars2_221/7}).
yeccpars2_221(S, ')', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 227, Ss, Stack, T, Ts, Tzr);
yeccpars2_221(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_222/7}).
-compile({nowarn_unused_function,  yeccpars2_222/7}).
yeccpars2_222(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 19, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 30, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_222(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_222_(Stack),
 yeccgoto_InputValueDefinitionList(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

%% yeccpars2_223: see yeccpars2_21

-dialyzer({nowarn_function, yeccpars2_224/7}).
-compile({nowarn_unused_function,  yeccpars2_224/7}).
yeccpars2_224(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 19, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 30, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_224(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_224_(Stack),
 yeccgoto_InputValueDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_225/7}).
-compile({nowarn_unused_function,  yeccpars2_225/7}).
yeccpars2_225(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_225_(Stack),
 yeccgoto_InputValueDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_226/7}).
-compile({nowarn_unused_function,  yeccpars2_226/7}).
yeccpars2_226(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_226_(Stack),
 yeccgoto_InputValueDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_227/7}).
-compile({nowarn_unused_function,  yeccpars2_227/7}).
yeccpars2_227(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_227_(Stack),
 yeccgoto_ArgumentsDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_228: see yeccpars2_209

-dialyzer({nowarn_function, yeccpars2_229/7}).
-compile({nowarn_unused_function,  yeccpars2_229/7}).
yeccpars2_229(S, '=', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 232, Ss, Stack, T, Ts, Tzr);
yeccpars2_229(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_229(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_229_(Stack),
 yeccgoto_InputValueDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_230/7}).
-compile({nowarn_unused_function,  yeccpars2_230/7}).
yeccpars2_230(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_230_(Stack),
 yeccgoto_InputValueDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_231/7}).
-compile({nowarn_unused_function,  yeccpars2_231/7}).
yeccpars2_231(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_231(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_231_(Stack),
 yeccgoto_InputValueDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_232: see yeccpars2_151

-dialyzer({nowarn_function, yeccpars2_233/7}).
-compile({nowarn_unused_function,  yeccpars2_233/7}).
yeccpars2_233(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_233_(Stack),
 yeccgoto_DefaultValue(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_234/7}).
-compile({nowarn_unused_function,  yeccpars2_234/7}).
yeccpars2_234(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_234_(Stack),
 yeccgoto_InputValueDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_235: see yeccpars2_209

-dialyzer({nowarn_function, yeccpars2_236/7}).
-compile({nowarn_unused_function,  yeccpars2_236/7}).
yeccpars2_236(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_236(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_236_(Stack),
 yeccgoto_FieldDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_237/7}).
-compile({nowarn_unused_function,  yeccpars2_237/7}).
yeccpars2_237(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_237_(Stack),
 yeccgoto_FieldDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_238: see yeccpars2_209

-dialyzer({nowarn_function, yeccpars2_239/7}).
-compile({nowarn_unused_function,  yeccpars2_239/7}).
yeccpars2_239(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_239_(Stack),
 yeccgoto_FieldDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_240/7}).
-compile({nowarn_unused_function,  yeccpars2_240/7}).
yeccpars2_240(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_240_(Stack),
 yeccgoto_ImplementsInterfaces(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_241/7}).
-compile({nowarn_unused_function,  yeccpars2_241/7}).
yeccpars2_241(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_241(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_241_(Stack),
 yeccgoto_NamedTypeList(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_242/7}).
-compile({nowarn_unused_function,  yeccpars2_242/7}).
yeccpars2_242(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_242_(Stack),
 yeccgoto_NamedTypeList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_243: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_244/7}).
-compile({nowarn_unused_function,  yeccpars2_244/7}).
yeccpars2_244(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 245, Ss, Stack, T, Ts, Tzr);
yeccpars2_244(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_245/7}).
-compile({nowarn_unused_function,  yeccpars2_245/7}).
yeccpars2_245(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_245_(Stack),
 yeccgoto_ObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_246/7}).
-compile({nowarn_unused_function,  yeccpars2_246/7}).
yeccpars2_246(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 250, Ss, Stack, T, Ts, Tzr);
yeccpars2_246(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_247: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_248/7}).
-compile({nowarn_unused_function,  yeccpars2_248/7}).
yeccpars2_248(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 249, Ss, Stack, T, Ts, Tzr);
yeccpars2_248(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_249/7}).
-compile({nowarn_unused_function,  yeccpars2_249/7}).
yeccpars2_249(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_249_(Stack),
 yeccgoto_ObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_250: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_251/7}).
-compile({nowarn_unused_function,  yeccpars2_251/7}).
yeccpars2_251(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 252, Ss, Stack, T, Ts, Tzr);
yeccpars2_251(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_252/7}).
-compile({nowarn_unused_function,  yeccpars2_252/7}).
yeccpars2_252(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_252_(Stack),
 yeccgoto_ObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_253/7}).
-compile({nowarn_unused_function,  yeccpars2_253/7}).
yeccpars2_253(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 257, Ss, Stack, T, Ts, Tzr);
yeccpars2_253(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_253_(Stack),
 yeccgoto_SchemaDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_254: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_255/7}).
-compile({nowarn_unused_function,  yeccpars2_255/7}).
yeccpars2_255(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 256, Ss, Stack, T, Ts, Tzr);
yeccpars2_255(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_256/7}).
-compile({nowarn_unused_function,  yeccpars2_256/7}).
yeccpars2_256(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_256_(Stack),
 yeccgoto_SchemaDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_257: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_258/7}).
-compile({nowarn_unused_function,  yeccpars2_258/7}).
yeccpars2_258(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 259, Ss, Stack, T, Ts, Tzr);
yeccpars2_258(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_259/7}).
-compile({nowarn_unused_function,  yeccpars2_259/7}).
yeccpars2_259(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_259_(Stack),
 yeccgoto_SchemaDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_260/7}).
-compile({nowarn_unused_function,  yeccpars2_260/7}).
yeccpars2_260(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_260(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_260_(Stack),
 yeccgoto_ScalarTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_261/7}).
-compile({nowarn_unused_function,  yeccpars2_261/7}).
yeccpars2_261(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_261_(Stack),
 yeccgoto_ScalarTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_262/7}).
-compile({nowarn_unused_function,  yeccpars2_262/7}).
yeccpars2_262(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_262(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 196, Ss, Stack, T, Ts, Tzr);
yeccpars2_262(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 265, Ss, Stack, T, Ts, Tzr);
yeccpars2_262(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_262_(Stack),
 yeccgoto_InterfaceTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_263/7}).
-compile({nowarn_unused_function,  yeccpars2_263/7}).
yeccpars2_263(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_263(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 272, Ss, Stack, T, Ts, Tzr);
yeccpars2_263(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_264/7}).
-compile({nowarn_unused_function,  yeccpars2_264/7}).
yeccpars2_264(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 268, Ss, Stack, T, Ts, Tzr);
yeccpars2_264(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_264_(Stack),
 yeccgoto_InterfaceTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_265: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_266/7}).
-compile({nowarn_unused_function,  yeccpars2_266/7}).
yeccpars2_266(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 267, Ss, Stack, T, Ts, Tzr);
yeccpars2_266(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_267/7}).
-compile({nowarn_unused_function,  yeccpars2_267/7}).
yeccpars2_267(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_267_(Stack),
 yeccgoto_InterfaceTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_268: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_269/7}).
-compile({nowarn_unused_function,  yeccpars2_269/7}).
yeccpars2_269(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 270, Ss, Stack, T, Ts, Tzr);
yeccpars2_269(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_270/7}).
-compile({nowarn_unused_function,  yeccpars2_270/7}).
yeccpars2_270(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_270_(Stack),
 yeccgoto_InterfaceTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_271/7}).
-compile({nowarn_unused_function,  yeccpars2_271/7}).
yeccpars2_271(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 275, Ss, Stack, T, Ts, Tzr);
yeccpars2_271(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_272: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_273/7}).
-compile({nowarn_unused_function,  yeccpars2_273/7}).
yeccpars2_273(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 274, Ss, Stack, T, Ts, Tzr);
yeccpars2_273(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_274/7}).
-compile({nowarn_unused_function,  yeccpars2_274/7}).
yeccpars2_274(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_274_(Stack),
 yeccgoto_InterfaceTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_275: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_276/7}).
-compile({nowarn_unused_function,  yeccpars2_276/7}).
yeccpars2_276(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 277, Ss, Stack, T, Ts, Tzr);
yeccpars2_276(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_277/7}).
-compile({nowarn_unused_function,  yeccpars2_277/7}).
yeccpars2_277(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_277_(Stack),
 yeccgoto_InterfaceTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_278/7}).
-compile({nowarn_unused_function,  yeccpars2_278/7}).
yeccpars2_278(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_278(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 280, Ss, Stack, T, Ts, Tzr);
yeccpars2_278(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_278_(Stack),
 yeccgoto_InputObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_279/7}).
-compile({nowarn_unused_function,  yeccpars2_279/7}).
yeccpars2_279(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 283, Ss, Stack, T, Ts, Tzr);
yeccpars2_279(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_279_(Stack),
 yeccgoto_InputObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_280: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_281/7}).
-compile({nowarn_unused_function,  yeccpars2_281/7}).
yeccpars2_281(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 282, Ss, Stack, T, Ts, Tzr);
yeccpars2_281(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_282/7}).
-compile({nowarn_unused_function,  yeccpars2_282/7}).
yeccpars2_282(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_282_(Stack),
 yeccgoto_InputObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_283: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_284/7}).
-compile({nowarn_unused_function,  yeccpars2_284/7}).
yeccpars2_284(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 285, Ss, Stack, T, Ts, Tzr);
yeccpars2_284(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_285/7}).
-compile({nowarn_unused_function,  yeccpars2_285/7}).
yeccpars2_285(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_285_(Stack),
 yeccgoto_InputObjectTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_286/7}).
-compile({nowarn_unused_function,  yeccpars2_286/7}).
yeccpars2_286(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 287, Ss, Stack, T, Ts, Tzr);
yeccpars2_286(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_287: see yeccpars2_21

%% yeccpars2_288: see yeccpars2_67

-dialyzer({nowarn_function, yeccpars2_289/7}).
-compile({nowarn_unused_function,  yeccpars2_289/7}).
yeccpars2_289(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_289_(Stack),
 yeccgoto_Fragment(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_290: see yeccpars2_63

-dialyzer({nowarn_function, yeccpars2_291/7}).
-compile({nowarn_unused_function,  yeccpars2_291/7}).
yeccpars2_291(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_291_(Stack),
 yeccgoto_Fragment(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_292/7}).
-compile({nowarn_unused_function,  yeccpars2_292/7}).
yeccpars2_292(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_292_(Stack),
 yeccgoto_TypeExtensionDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_293/7}).
-compile({nowarn_unused_function,  yeccpars2_293/7}).
yeccpars2_293(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_293_(Stack),
 yeccgoto_TypeExtensionDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_294/7}).
-compile({nowarn_unused_function,  yeccpars2_294/7}).
yeccpars2_294(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_294_(Stack),
 yeccgoto_TypeExtensionDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_295/7}).
-compile({nowarn_unused_function,  yeccpars2_295/7}).
yeccpars2_295(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_295_(Stack),
 yeccgoto_TypeExtensionDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_296/7}).
-compile({nowarn_unused_function,  yeccpars2_296/7}).
yeccpars2_296(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_296_(Stack),
 yeccgoto_TypeExtensionDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_297/7}).
-compile({nowarn_unused_function,  yeccpars2_297/7}).
yeccpars2_297(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_297_(Stack),
 yeccgoto_TypeExtensionDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_298/7}).
-compile({nowarn_unused_function,  yeccpars2_298/7}).
yeccpars2_298(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_298_(Stack),
 yeccgoto_TypeExtensionDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_299/7}).
-compile({nowarn_unused_function,  yeccpars2_299/7}).
yeccpars2_299(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_299(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 301, Ss, Stack, T, Ts, Tzr);
yeccpars2_299(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_299_(Stack),
 yeccgoto_EnumTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_300/7}).
-compile({nowarn_unused_function,  yeccpars2_300/7}).
yeccpars2_300(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 311, Ss, Stack, T, Ts, Tzr);
yeccpars2_300(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_300_(Stack),
 yeccgoto_EnumTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_301: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_302/7}).
-compile({nowarn_unused_function,  yeccpars2_302/7}).
yeccpars2_302(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 310, Ss, Stack, T, Ts, Tzr);
yeccpars2_302(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_303/7}).
-compile({nowarn_unused_function,  yeccpars2_303/7}).
yeccpars2_303(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 19, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 30, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_303(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_303_(Stack),
 yeccgoto_EnumValueDefinitionList(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_304/7}).
-compile({nowarn_unused_function,  yeccpars2_304/7}).
yeccpars2_304(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_304(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_304_(Stack),
 yeccgoto_EnumValueDefinition(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

%% yeccpars2_305: see yeccpars2_21

-dialyzer({nowarn_function, yeccpars2_306/7}).
-compile({nowarn_unused_function,  yeccpars2_306/7}).
yeccpars2_306(S, 'block_string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 19, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'directive', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 44, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'enum', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 45, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'extend', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 46, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'fragment', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 47, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'implements', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 48, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'input', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 49, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'interface', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 50, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'mutation', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 51, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'name', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 52, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'query', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 54, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'scalar', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 55, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'schema', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 56, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'string_value', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 30, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'subscription', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 57, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'type', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 58, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(S, 'union', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 59, Ss, Stack, T, Ts, Tzr);
yeccpars2_306(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_306_(Stack),
 yeccgoto_EnumValueDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_307/7}).
-compile({nowarn_unused_function,  yeccpars2_307/7}).
yeccpars2_307(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_307_(Stack),
 yeccgoto_EnumValueDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_308/7}).
-compile({nowarn_unused_function,  yeccpars2_308/7}).
yeccpars2_308(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_308_(Stack),
 yeccgoto_EnumValueDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_309/7}).
-compile({nowarn_unused_function,  yeccpars2_309/7}).
yeccpars2_309(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_309_(Stack),
 yeccgoto_EnumValueDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_310/7}).
-compile({nowarn_unused_function,  yeccpars2_310/7}).
yeccpars2_310(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_310_(Stack),
 yeccgoto_EnumTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_311: see yeccpars2_197

-dialyzer({nowarn_function, yeccpars2_312/7}).
-compile({nowarn_unused_function,  yeccpars2_312/7}).
yeccpars2_312(S, '}', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 313, Ss, Stack, T, Ts, Tzr);
yeccpars2_312(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_313/7}).
-compile({nowarn_unused_function,  yeccpars2_313/7}).
yeccpars2_313(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_313_(Stack),
 yeccgoto_EnumTypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_314: see yeccpars2_21

-dialyzer({nowarn_function, yeccpars2_315/7}).
-compile({nowarn_unused_function,  yeccpars2_315/7}).
yeccpars2_315(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 208, Ss, Stack, T, Ts, Tzr);
yeccpars2_315(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 317, Ss, Stack, T, Ts, Tzr);
yeccpars2_315(S, 'repeatable', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 318, Ss, Stack, T, Ts, Tzr);
yeccpars2_315(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_316/7}).
-compile({nowarn_unused_function,  yeccpars2_316/7}).
yeccpars2_316(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 331, Ss, Stack, T, Ts, Tzr);
yeccpars2_316(S, 'repeatable', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 332, Ss, Stack, T, Ts, Tzr);
yeccpars2_316(_, _, _, _, T, _, _) ->
 yeccerror(T).

yeccpars2_317(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 53, Ss, Stack, T, Ts, Tzr);
yeccpars2_317(S, '|', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 322, Ss, Stack, T, Ts, Tzr);
yeccpars2_317(S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_23(S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_318/7}).
-compile({nowarn_unused_function,  yeccpars2_318/7}).
yeccpars2_318(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 319, Ss, Stack, T, Ts, Tzr);
yeccpars2_318(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_319: see yeccpars2_317

-dialyzer({nowarn_function, yeccpars2_320/7}).
-compile({nowarn_unused_function,  yeccpars2_320/7}).
yeccpars2_320(S, '|', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 327, Ss, Stack, T, Ts, Tzr);
yeccpars2_320(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_320_(Stack),
 yeccgoto_DirectiveDefinitionLocations(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_321/7}).
-compile({nowarn_unused_function,  yeccpars2_321/7}).
yeccpars2_321(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_321(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_321_(Stack),
 yeccgoto_DirectiveDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_322: see yeccpars2_21

-dialyzer({nowarn_function, yeccpars2_323/7}).
-compile({nowarn_unused_function,  yeccpars2_323/7}).
yeccpars2_323(S, '|', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 324, Ss, Stack, T, Ts, Tzr);
yeccpars2_323(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_324: see yeccpars2_317

-dialyzer({nowarn_function, yeccpars2_325/7}).
-compile({nowarn_unused_function,  yeccpars2_325/7}).
yeccpars2_325(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_325_(Stack),
 yeccgoto_DirectiveDefinitionLocations(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_326/7}).
-compile({nowarn_unused_function,  yeccpars2_326/7}).
yeccpars2_326(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_326_(Stack),
 yeccgoto_DirectiveDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_327: see yeccpars2_317

-dialyzer({nowarn_function, yeccpars2_328/7}).
-compile({nowarn_unused_function,  yeccpars2_328/7}).
yeccpars2_328(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_328_(Stack),
 yeccgoto_DirectiveDefinitionLocations(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_329/7}).
-compile({nowarn_unused_function,  yeccpars2_329/7}).
yeccpars2_329(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_329(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_329_(Stack),
 yeccgoto_DirectiveDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_330/7}).
-compile({nowarn_unused_function,  yeccpars2_330/7}).
yeccpars2_330(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_330_(Stack),
 yeccgoto_DirectiveDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_331: see yeccpars2_317

-dialyzer({nowarn_function, yeccpars2_332/7}).
-compile({nowarn_unused_function,  yeccpars2_332/7}).
yeccpars2_332(S, 'on', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 333, Ss, Stack, T, Ts, Tzr);
yeccpars2_332(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_333: see yeccpars2_317

-dialyzer({nowarn_function, yeccpars2_334/7}).
-compile({nowarn_unused_function,  yeccpars2_334/7}).
yeccpars2_334(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_334(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_334_(Stack),
 yeccgoto_DirectiveDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_335/7}).
-compile({nowarn_unused_function,  yeccpars2_335/7}).
yeccpars2_335(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_335_(Stack),
 yeccgoto_DirectiveDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_336/7}).
-compile({nowarn_unused_function,  yeccpars2_336/7}).
yeccpars2_336(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_336(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_336_(Stack),
 yeccgoto_DirectiveDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_337/7}).
-compile({nowarn_unused_function,  yeccpars2_337/7}).
yeccpars2_337(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_337_(Stack),
 yeccgoto_DirectiveDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_338/7}).
-compile({nowarn_unused_function,  yeccpars2_338/7}).
yeccpars2_338(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_338_(Stack),
 yeccgoto_Definitions(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_339/7}).
-compile({nowarn_unused_function,  yeccpars2_339/7}).
yeccpars2_339(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_339_(Stack),
 yeccgoto_TypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_340/7}).
-compile({nowarn_unused_function,  yeccpars2_340/7}).
yeccpars2_340(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_340_(Stack),
 yeccgoto_TypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_341/7}).
-compile({nowarn_unused_function,  yeccpars2_341/7}).
yeccpars2_341(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_341_(Stack),
 yeccgoto_TypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_342/7}).
-compile({nowarn_unused_function,  yeccpars2_342/7}).
yeccpars2_342(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_342_(Stack),
 yeccgoto_Definition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_343/7}).
-compile({nowarn_unused_function,  yeccpars2_343/7}).
yeccpars2_343(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_343_(Stack),
 yeccgoto_TypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_344/7}).
-compile({nowarn_unused_function,  yeccpars2_344/7}).
yeccpars2_344(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_344_(Stack),
 yeccgoto_TypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_345/7}).
-compile({nowarn_unused_function,  yeccpars2_345/7}).
yeccpars2_345(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_345_(Stack),
 yeccgoto_TypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_346/7}).
-compile({nowarn_unused_function,  yeccpars2_346/7}).
yeccpars2_346(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_346_(Stack),
 yeccgoto_Definition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_347/7}).
-compile({nowarn_unused_function,  yeccpars2_347/7}).
yeccpars2_347(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_347_(Stack),
 yeccgoto_TypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_348/7}).
-compile({nowarn_unused_function,  yeccpars2_348/7}).
yeccpars2_348(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_348_(Stack),
 yeccgoto_TypeDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_349: see yeccpars2_67

-dialyzer({nowarn_function, yeccpars2_350/7}).
-compile({nowarn_unused_function,  yeccpars2_350/7}).
yeccpars2_350(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_350_(Stack),
 yeccgoto_OperationDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_351/7}).
-compile({nowarn_unused_function,  yeccpars2_351/7}).
yeccpars2_351(S, '(', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 353, Ss, Stack, T, Ts, Tzr);
yeccpars2_351(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 65, Ss, Stack, T, Ts, Tzr);
yeccpars2_351(S, '{', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 34, Ss, Stack, T, Ts, Tzr);
yeccpars2_351(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_352: see yeccpars2_63

-dialyzer({nowarn_function, yeccpars2_353/7}).
-compile({nowarn_unused_function,  yeccpars2_353/7}).
yeccpars2_353(S, '$', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 88, Ss, Stack, T, Ts, Tzr);
yeccpars2_353(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_354/7}).
-compile({nowarn_unused_function,  yeccpars2_354/7}).
yeccpars2_354(S, ')', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 363, Ss, Stack, T, Ts, Tzr);
yeccpars2_354(_, _, _, _, T, _, _) ->
 yeccerror(T).

-dialyzer({nowarn_function, yeccpars2_355/7}).
-compile({nowarn_unused_function,  yeccpars2_355/7}).
yeccpars2_355(S, '$', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 88, Ss, Stack, T, Ts, Tzr);
yeccpars2_355(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 NewStack = yeccpars2_355_(Stack),
 yeccgoto_VariableDefinitionList(hd(Ss), Cat, Ss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_356/7}).
-compile({nowarn_unused_function,  yeccpars2_356/7}).
yeccpars2_356(S, ':', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 357, Ss, Stack, T, Ts, Tzr);
yeccpars2_356(_, _, _, _, T, _, _) ->
 yeccerror(T).

%% yeccpars2_357: see yeccpars2_209

-dialyzer({nowarn_function, yeccpars2_358/7}).
-compile({nowarn_unused_function,  yeccpars2_358/7}).
yeccpars2_358(S, '=', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 232, Ss, Stack, T, Ts, Tzr);
yeccpars2_358(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_358(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_358_(Stack),
 yeccgoto_VariableDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_359/7}).
-compile({nowarn_unused_function,  yeccpars2_359/7}).
yeccpars2_359(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_359_(Stack),
 yeccgoto_VariableDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_360/7}).
-compile({nowarn_unused_function,  yeccpars2_360/7}).
yeccpars2_360(S, '@', Ss, Stack, T, Ts, Tzr) ->
 yeccpars1(S, 142, Ss, Stack, T, Ts, Tzr);
yeccpars2_360(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_360_(Stack),
 yeccgoto_VariableDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_361/7}).
-compile({nowarn_unused_function,  yeccpars2_361/7}).
yeccpars2_361(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_361_(Stack),
 yeccgoto_VariableDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_362/7}).
-compile({nowarn_unused_function,  yeccpars2_362/7}).
yeccpars2_362(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_|Nss] = Ss,
 NewStack = yeccpars2_362_(Stack),
 yeccgoto_VariableDefinitionList(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_363/7}).
-compile({nowarn_unused_function,  yeccpars2_363/7}).
yeccpars2_363(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_363_(Stack),
 yeccgoto_VariableDefinitions(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_364/7}).
-compile({nowarn_unused_function,  yeccpars2_364/7}).
yeccpars2_364(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_364_(Stack),
 yeccgoto_OperationDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_365: see yeccpars2_67

-dialyzer({nowarn_function, yeccpars2_366/7}).
-compile({nowarn_unused_function,  yeccpars2_366/7}).
yeccpars2_366(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_366_(Stack),
 yeccgoto_OperationDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_367: see yeccpars2_63

-dialyzer({nowarn_function, yeccpars2_368/7}).
-compile({nowarn_unused_function,  yeccpars2_368/7}).
yeccpars2_368(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_368_(Stack),
 yeccgoto_OperationDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_369/7}).
-compile({nowarn_unused_function,  yeccpars2_369/7}).
yeccpars2_369(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_369_(Stack),
 yeccgoto_OperationDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_370: see yeccpars2_63

-dialyzer({nowarn_function, yeccpars2_371/7}).
-compile({nowarn_unused_function,  yeccpars2_371/7}).
yeccpars2_371(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_,_|Nss] = Ss,
 NewStack = yeccpars2_371_(Stack),
 yeccgoto_OperationDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccpars2_372/7}).
-compile({nowarn_unused_function,  yeccpars2_372/7}).
yeccpars2_372(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_|Nss] = Ss,
 NewStack = yeccpars2_372_(Stack),
 yeccgoto_OperationDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

%% yeccpars2_373: see yeccpars2_63

-dialyzer({nowarn_function, yeccpars2_374/7}).
-compile({nowarn_unused_function,  yeccpars2_374/7}).
yeccpars2_374(_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 [_,_,_|Nss] = Ss,
 NewStack = yeccpars2_374_(Stack),
 yeccgoto_OperationDefinition(hd(Nss), Cat, Nss, NewStack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Alias/7}).
-compile({nowarn_unused_function,  yeccgoto_Alias/7}).
yeccgoto_Alias(34, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(42, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Alias(36, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(42, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Argument/7}).
-compile({nowarn_unused_function,  yeccgoto_Argument/7}).
yeccgoto_Argument(76, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_79(79, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Argument(79, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_79(79, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ArgumentConst/7}).
-compile({nowarn_unused_function,  yeccgoto_ArgumentConst/7}).
yeccgoto_ArgumentConst(146, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_149(149, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ArgumentConst(149, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_149(149, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ArgumentList/7}).
-compile({nowarn_unused_function,  yeccgoto_ArgumentList/7}).
yeccgoto_ArgumentList(76, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_78(78, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ArgumentList(79=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_112(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ArgumentListConst/7}).
-compile({nowarn_unused_function,  yeccgoto_ArgumentListConst/7}).
yeccgoto_ArgumentListConst(146, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_148(148, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ArgumentListConst(149=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_177(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Arguments/7}).
-compile({nowarn_unused_function,  yeccgoto_Arguments/7}).
yeccgoto_Arguments(38, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_130(130, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Arguments(73=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_116(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Arguments(74=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_75(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Arguments(120, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_123(123, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ArgumentsConst/7}).
-compile({nowarn_unused_function,  yeccgoto_ArgumentsConst/7}).
yeccgoto_ArgumentsConst(143=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_181(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ArgumentsConst(144=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_145(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ArgumentsDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_ArgumentsDefinition/7}).
yeccgoto_ArgumentsDefinition(198, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_207(207, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ArgumentsDefinition(315, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_316(316, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_DefaultValue/7}).
-compile({nowarn_unused_function,  yeccgoto_DefaultValue/7}).
yeccgoto_DefaultValue(229, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_231(231, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DefaultValue(358, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_360(360, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Definition/7}).
-compile({nowarn_unused_function,  yeccgoto_Definition/7}).
yeccgoto_Definition(0, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_18(18, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Definition(18, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_18(18, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Definitions/7}).
-compile({nowarn_unused_function,  yeccgoto_Definitions/7}).
yeccgoto_Definitions(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_17(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Definitions(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_338(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_DescriptionDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_DescriptionDefinition/7}).
yeccgoto_DescriptionDefinition(0, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_16(16, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(18, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_16(16, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(197, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(200, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(202, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(208, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(223, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(222, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(223, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(224, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(223, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(243, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(247, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(250, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(254, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(257, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(265, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(268, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(272, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(275, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(201, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(280, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(223, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(283, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(223, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(301, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(305, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(303, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(305, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(306, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(305, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DescriptionDefinition(311, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_21(305, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Directive/7}).
-compile({nowarn_unused_function,  yeccgoto_Directive/7}).
yeccgoto_Directive(7, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(38, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(43, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(62, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(64, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(67, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(120, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(123, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(130, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(288, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(321, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(329, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(334, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(336, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(349, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(351, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directive(365, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_64(64, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_DirectiveConst/7}).
-compile({nowarn_unused_function,  yeccgoto_DirectiveConst/7}).
yeccgoto_DirectiveConst(29, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(138, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(140, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(193, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(194, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(198, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(210, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(229, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(231, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(236, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(260, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(262, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(263, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(278, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(299, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(304, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(358, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveConst(360, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_140(140, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_DirectiveDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_DirectiveDefinition/7}).
yeccgoto_DirectiveDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_15(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveDefinition(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_348(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_15(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_DirectiveDefinitionLocations/7}).
-compile({nowarn_unused_function,  yeccgoto_DirectiveDefinitionLocations/7}).
yeccgoto_DirectiveDefinitionLocations(317, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_329(329, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveDefinitionLocations(319, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_321(321, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveDefinitionLocations(324=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_325(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveDefinitionLocations(327=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_328(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveDefinitionLocations(331, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_336(336, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectiveDefinitionLocations(333, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_334(334, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Directives/7}).
-compile({nowarn_unused_function,  yeccgoto_Directives/7}).
yeccgoto_Directives(7, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_63(352, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(38, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_129(129, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(43, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_63(63, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(62=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_119(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(64=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_117(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(67, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_63(71, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(120, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_122(122, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(123, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_125(125, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(130, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_133(133, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(288, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_63(290, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(321=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_326(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(329=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_330(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(334=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_335(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(336=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_337(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(349, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_63(373, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(351, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_63(367, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Directives(365, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_63(370, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_DirectivesConst/7}).
-compile({nowarn_unused_function,  yeccgoto_DirectivesConst/7}).
yeccgoto_DirectivesConst(29, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_253(253, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(138, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_139(139, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(140=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_190(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(193, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_195(195, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(194, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_246(246, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(198, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_206(206, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(210=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_219(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(229=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_230(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(231=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_234(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(236=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_237(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(260=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_261(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(262, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_264(264, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(263, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_271(271, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(278, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_279(279, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(299, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_300(300, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(304=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_308(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(358=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_359(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_DirectivesConst(360=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_361(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Document/7}).
-compile({nowarn_unused_function,  yeccgoto_Document/7}).
yeccgoto_Document(0, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_14(14, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_EnumTypeDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_EnumTypeDefinition/7}).
yeccgoto_EnumTypeDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_13(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumTypeDefinition(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_347(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumTypeDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_13(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumTypeDefinition(22=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_298(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_EnumValue/7}).
-compile({nowarn_unused_function,  yeccgoto_EnumValue/7}).
yeccgoto_EnumValue(81=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_87(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(89=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_87(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(101=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_87(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(106=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_87(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(114=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_87(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(151=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_155(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(156=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_155(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(168=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_155(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(173=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_155(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(179=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_155(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(232=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_155(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(301, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_304(304, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(303, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_304(304, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(305, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_304(304, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(306, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_304(304, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValue(311, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_304(304, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_EnumValueDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_EnumValueDefinition/7}).
yeccgoto_EnumValueDefinition(301, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_303(303, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValueDefinition(303, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_303(303, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValueDefinition(305, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_306(306, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValueDefinition(306, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_303(303, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValueDefinition(311, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_303(303, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_EnumValueDefinitionList/7}).
-compile({nowarn_unused_function,  yeccgoto_EnumValueDefinitionList/7}).
yeccgoto_EnumValueDefinitionList(301, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_302(302, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValueDefinitionList(303=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_309(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValueDefinitionList(306=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_307(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_EnumValueDefinitionList(311, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_312(312, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Field/7}).
-compile({nowarn_unused_function,  yeccgoto_Field/7}).
yeccgoto_Field(34=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_41(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Field(36=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_41(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_FieldDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_FieldDefinition/7}).
yeccgoto_FieldDefinition(197, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(200, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(201, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_202(202, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(202, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(243, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(247, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(250, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(254, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(257, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(265, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(268, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(272, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinition(275, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_200(200, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_FieldDefinitionList/7}).
-compile({nowarn_unused_function,  yeccgoto_FieldDefinitionList/7}).
yeccgoto_FieldDefinitionList(197, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_199(199, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(200=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_204(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(202=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_203(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(243, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_244(244, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(247, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_248(248, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(250, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_251(251, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(254, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_255(255, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(257, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_258(258, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(265, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_266(266, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(268, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_269(269, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(272, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_273(273, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FieldDefinitionList(275, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_276(276, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Fragment/7}).
-compile({nowarn_unused_function,  yeccgoto_Fragment/7}).
yeccgoto_Fragment(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_12(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Fragment(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_346(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Fragment(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_12(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_FragmentName/7}).
-compile({nowarn_unused_function,  yeccgoto_FragmentName/7}).
yeccgoto_FragmentName(23, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_286(286, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FragmentName(43, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_62(62, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_FragmentSpread/7}).
-compile({nowarn_unused_function,  yeccgoto_FragmentSpread/7}).
yeccgoto_FragmentSpread(34=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_40(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_FragmentSpread(36=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_40(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ImplementsInterfaces/7}).
-compile({nowarn_unused_function,  yeccgoto_ImplementsInterfaces/7}).
yeccgoto_ImplementsInterfaces(193, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_194(194, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ImplementsInterfaces(262, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_263(263, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_InlineFragment/7}).
-compile({nowarn_unused_function,  yeccgoto_InlineFragment/7}).
yeccgoto_InlineFragment(34=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_39(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InlineFragment(36=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_39(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_InputObjectTypeDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_InputObjectTypeDefinition/7}).
yeccgoto_InputObjectTypeDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_11(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputObjectTypeDefinition(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_345(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputObjectTypeDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_11(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputObjectTypeDefinition(22=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_297(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_InputValueDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_InputValueDefinition/7}).
yeccgoto_InputValueDefinition(208, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_222(222, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputValueDefinition(222, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_222(222, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputValueDefinition(223, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_224(224, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputValueDefinition(224, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_222(222, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputValueDefinition(280, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_222(222, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputValueDefinition(283, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_222(222, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_InputValueDefinitionList/7}).
-compile({nowarn_unused_function,  yeccgoto_InputValueDefinitionList/7}).
yeccgoto_InputValueDefinitionList(208, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_221(221, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputValueDefinitionList(222=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_226(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputValueDefinitionList(224=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_225(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputValueDefinitionList(280, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_281(281, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InputValueDefinitionList(283, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_284(284, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_InterfaceTypeDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_InterfaceTypeDefinition/7}).
yeccgoto_InterfaceTypeDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_10(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InterfaceTypeDefinition(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_344(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InterfaceTypeDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_10(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_InterfaceTypeDefinition(22=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_296(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ListType/7}).
-compile({nowarn_unused_function,  yeccgoto_ListType/7}).
yeccgoto_ListType(209, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_213(213, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListType(214, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_213(213, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListType(228, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_213(213, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListType(235, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_213(213, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListType(238, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_213(213, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListType(357, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_213(213, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ListValue/7}).
-compile({nowarn_unused_function,  yeccgoto_ListValue/7}).
yeccgoto_ListValue(81=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_86(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListValue(89=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_86(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListValue(101=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_86(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListValue(106=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_86(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListValue(114=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_86(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ListValueConst/7}).
-compile({nowarn_unused_function,  yeccgoto_ListValueConst/7}).
yeccgoto_ListValueConst(151=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_154(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListValueConst(156=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_154(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListValueConst(168=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_154(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListValueConst(173=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_154(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListValueConst(179=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_154(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ListValueConst(232=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_154(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Name/7}).
-compile({nowarn_unused_function,  yeccgoto_Name/7}).
yeccgoto_Name(7, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_351(351, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(21, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_299(299, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(24, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_278(278, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(25, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_262(262, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(28, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_260(260, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(32, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_193(193, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(33, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_138(138, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(34, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_38(38, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(36, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_38(38, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(42, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_120(120, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(66=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(81=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(89=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(96, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_99(99, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(98, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_99(99, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(101=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(106=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(114=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(141=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(151=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(156=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(163, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_166(166, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(165, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_166(166, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(168=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(173=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(179=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(184=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(186=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(188=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(191=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(196=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(197, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(200, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(201, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(202, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(208, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_220(220, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(209=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(214=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(222, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_220(220, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(223, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_220(220, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(224, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_220(220, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(228=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(232=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(235=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(238=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(241=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(243, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(247, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(250, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(254, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(257, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(265, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(268, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(272, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(275, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_198(198, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(280, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_220(220, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(283, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_220(220, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(287=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(301=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(303=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(305=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(306=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(311=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_85(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(314, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_315(315, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(317, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_320(320, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(319, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_320(320, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(322, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_323(323, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(324, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_320(320, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(327, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_320(320, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(331, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_320(320, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(333, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_320(320, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Name(357=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_69(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_NameWithoutOn/7}).
-compile({nowarn_unused_function,  yeccgoto_NameWithoutOn/7}).
yeccgoto_NameWithoutOn(7=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(21=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(23=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_61(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(24=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(25=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(28=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(32=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(33=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(34=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(36=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(42=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(43=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_61(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(65, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_73(73, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(66=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(76, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_77(77, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(79, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_77(77, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(81=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(88=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_110(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(89=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(96=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(98=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(101=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(106=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(114=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(141=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(142, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_143(143, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(146, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_147(147, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(149, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_147(147, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(151=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(156=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(163=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(165=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(168=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(173=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(179=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(184=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(186=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(188=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(191=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(196=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(197=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(200=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(201=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(202=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(208=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(209=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(214=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(222=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(223=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(224=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(228=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(232=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(235=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(238=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(241=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(243=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(247=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(250=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(254=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(257=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(265=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(268=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(272=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(275=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(280=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(283=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(287=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(301=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(303=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(305=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(306=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(311=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(314=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(317=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(319=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(322=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(324=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(327=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(331=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(333=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NameWithoutOn(357=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_37(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_NamedType/7}).
-compile({nowarn_unused_function,  yeccgoto_NamedType/7}).
yeccgoto_NamedType(66=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_68(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(141, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_183(183, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(184, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_185(185, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(186, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_183(183, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(188, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_183(183, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(191, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_183(183, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(196, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_241(241, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(209, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_212(212, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(214, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_212(212, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(228, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_212(212, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(235, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_212(212, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(238, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_212(212, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(241, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_241(241, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(287=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_68(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedType(357, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_212(212, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_NamedTypeList/7}).
-compile({nowarn_unused_function,  yeccgoto_NamedTypeList/7}).
yeccgoto_NamedTypeList(196=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_240(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NamedTypeList(241=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_242(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_NonNullType/7}).
-compile({nowarn_unused_function,  yeccgoto_NonNullType/7}).
yeccgoto_NonNullType(209=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_211(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NonNullType(214=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_211(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NonNullType(228=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_211(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NonNullType(235=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_211(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NonNullType(238=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_211(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_NonNullType(357=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_211(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ObjectField/7}).
-compile({nowarn_unused_function,  yeccgoto_ObjectField/7}).
yeccgoto_ObjectField(96, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_98(98, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectField(98, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_98(98, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ObjectFieldConst/7}).
-compile({nowarn_unused_function,  yeccgoto_ObjectFieldConst/7}).
yeccgoto_ObjectFieldConst(163, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_165(165, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectFieldConst(165, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_165(165, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ObjectFields/7}).
-compile({nowarn_unused_function,  yeccgoto_ObjectFields/7}).
yeccgoto_ObjectFields(96, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_97(97, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectFields(98=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_103(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ObjectFieldsConst/7}).
-compile({nowarn_unused_function,  yeccgoto_ObjectFieldsConst/7}).
yeccgoto_ObjectFieldsConst(163, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_164(164, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectFieldsConst(165=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_170(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ObjectTypeDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_ObjectTypeDefinition/7}).
yeccgoto_ObjectTypeDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_9(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectTypeDefinition(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_343(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectTypeDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_9(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectTypeDefinition(22=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_295(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ObjectValue/7}).
-compile({nowarn_unused_function,  yeccgoto_ObjectValue/7}).
yeccgoto_ObjectValue(81=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_84(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectValue(89=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_84(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectValue(101=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_84(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectValue(106=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_84(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectValue(114=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_84(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ObjectValueConst/7}).
-compile({nowarn_unused_function,  yeccgoto_ObjectValueConst/7}).
yeccgoto_ObjectValueConst(151=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_153(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectValueConst(156=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_153(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectValueConst(168=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_153(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectValueConst(173=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_153(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectValueConst(179=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_153(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ObjectValueConst(232=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_153(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_OperationDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_OperationDefinition/7}).
yeccgoto_OperationDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_8(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_OperationDefinition(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_342(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_OperationDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_8(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_OperationType/7}).
-compile({nowarn_unused_function,  yeccgoto_OperationType/7}).
yeccgoto_OperationType(0, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_7(7, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_OperationType(16, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_7(7, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_OperationType(18, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_7(7, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ScalarTypeDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_ScalarTypeDefinition/7}).
yeccgoto_ScalarTypeDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_6(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ScalarTypeDefinition(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_341(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ScalarTypeDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_6(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ScalarTypeDefinition(22=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_294(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_SchemaDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_SchemaDefinition/7}).
yeccgoto_SchemaDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_5(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SchemaDefinition(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_340(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SchemaDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_5(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SchemaDefinition(22=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_293(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Selection/7}).
-compile({nowarn_unused_function,  yeccgoto_Selection/7}).
yeccgoto_Selection(34, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_36(36, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Selection(36, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_36(36, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_SelectionSet/7}).
-compile({nowarn_unused_function,  yeccgoto_SelectionSet/7}).
yeccgoto_SelectionSet(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_4(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(7=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_350(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_4(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_4(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(38=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_128(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(43=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_60(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(63=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_118(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(67=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_70(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(71=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_72(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(120=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_121(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(122=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_127(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(123=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_124(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(125=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_126(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(129=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_135(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(130=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_132(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(133=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_134(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(288=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_289(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(290=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_291(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(349=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_372(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(351=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_366(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(352=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_364(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(365=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_369(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(367=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_368(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(370=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_371(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_SelectionSet(373=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_374(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Selections/7}).
-compile({nowarn_unused_function,  yeccgoto_Selections/7}).
yeccgoto_Selections(34, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_35(35, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Selections(36=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_136(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Type/7}).
-compile({nowarn_unused_function,  yeccgoto_Type/7}).
yeccgoto_Type(209, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_210(210, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Type(214, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_215(215, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Type(228, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_229(229, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Type(235, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_236(236, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Type(238=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_239(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Type(357, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_358(358, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_TypeCondition/7}).
-compile({nowarn_unused_function,  yeccgoto_TypeCondition/7}).
yeccgoto_TypeCondition(66, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_67(67, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_TypeCondition(287, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_67(288, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_TypeDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_TypeDefinition/7}).
yeccgoto_TypeDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_3(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_TypeDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_3(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_TypeExtensionDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_TypeExtensionDefinition/7}).
yeccgoto_TypeExtensionDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_2(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_TypeExtensionDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_2(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_UnionMembers/7}).
-compile({nowarn_unused_function,  yeccgoto_UnionMembers/7}).
yeccgoto_UnionMembers(141=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_182(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_UnionMembers(186=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_187(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_UnionMembers(188=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_189(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_UnionMembers(191=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_192(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_UnionTypeDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_UnionTypeDefinition/7}).
yeccgoto_UnionTypeDefinition(0=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_1(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_UnionTypeDefinition(16=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_339(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_UnionTypeDefinition(18=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_1(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_UnionTypeDefinition(22=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_292(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Value/7}).
-compile({nowarn_unused_function,  yeccgoto_Value/7}).
yeccgoto_Value(81=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_83(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Value(89, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_106(106, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Value(101=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_102(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Value(106, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_106(106, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Value(114=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_115(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ValueConst/7}).
-compile({nowarn_unused_function,  yeccgoto_ValueConst/7}).
yeccgoto_ValueConst(151=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_152(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ValueConst(156, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_173(173, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ValueConst(168=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_169(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ValueConst(173, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_173(173, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ValueConst(179=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_180(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ValueConst(232=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_233(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Values/7}).
-compile({nowarn_unused_function,  yeccgoto_Values/7}).
yeccgoto_Values(89, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_105(105, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Values(106=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_108(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_ValuesConst/7}).
-compile({nowarn_unused_function,  yeccgoto_ValuesConst/7}).
yeccgoto_ValuesConst(156, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_172(172, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_ValuesConst(173=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_175(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_Variable/7}).
-compile({nowarn_unused_function,  yeccgoto_Variable/7}).
yeccgoto_Variable(81=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_82(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Variable(89=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_82(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Variable(101=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_82(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Variable(106=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_82(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Variable(114=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_82(_S, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Variable(353, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_356(356, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_Variable(355, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_356(356, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_VariableDefinition/7}).
-compile({nowarn_unused_function,  yeccgoto_VariableDefinition/7}).
yeccgoto_VariableDefinition(353, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_355(355, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_VariableDefinition(355, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_355(355, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_VariableDefinitionList/7}).
-compile({nowarn_unused_function,  yeccgoto_VariableDefinitionList/7}).
yeccgoto_VariableDefinitionList(353, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_354(354, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_VariableDefinitionList(355=_S, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_362(_S, Cat, Ss, Stack, T, Ts, Tzr).

-dialyzer({nowarn_function, yeccgoto_VariableDefinitions/7}).
-compile({nowarn_unused_function,  yeccgoto_VariableDefinitions/7}).
yeccgoto_VariableDefinitions(7, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_67(349, Cat, Ss, Stack, T, Ts, Tzr);
yeccgoto_VariableDefinitions(351, Cat, Ss, Stack, T, Ts, Tzr) ->
 yeccpars2_67(365, Cat, Ss, Stack, T, Ts, Tzr).

-compile({inline,yeccpars2_1_/1}).
-dialyzer({nowarn_function, yeccpars2_1_/1}).
-compile({nowarn_unused_function,  yeccpars2_1_/1}).
-file("src/absinthe_parser.yrl", 209).
yeccpars2_1_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                        ___1
  end | __Stack].

-compile({inline,yeccpars2_2_/1}).
-dialyzer({nowarn_function, yeccpars2_2_/1}).
-compile({nowarn_unused_function,  yeccpars2_2_/1}).
-file("src/absinthe_parser.yrl", 213).
yeccpars2_2_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                            ___1
  end | __Stack].

-compile({inline,yeccpars2_3_/1}).
-dialyzer({nowarn_function, yeccpars2_3_/1}).
-compile({nowarn_unused_function,  yeccpars2_3_/1}).
-file("src/absinthe_parser.yrl", 33).
yeccpars2_3_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                               ___1
  end | __Stack].

-compile({inline,yeccpars2_4_/1}).
-dialyzer({nowarn_function, yeccpars2_4_/1}).
-compile({nowarn_unused_function,  yeccpars2_4_/1}).
-file("src/absinthe_parser.yrl", 39).
yeccpars2_4_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                      build_ast_node('OperationDefinition', #{'operation' => 'query', 'selection_set' => ___1, 'shorthand' => true}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_5_/1}).
-dialyzer({nowarn_function, yeccpars2_5_/1}).
-compile({nowarn_unused_function,  yeccpars2_5_/1}).
-file("src/absinthe_parser.yrl", 206).
yeccpars2_5_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                     ___1
  end | __Stack].

-compile({inline,yeccpars2_6_/1}).
-dialyzer({nowarn_function, yeccpars2_6_/1}).
-compile({nowarn_unused_function,  yeccpars2_6_/1}).
-file("src/absinthe_parser.yrl", 210).
yeccpars2_6_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                         ___1
  end | __Stack].

-compile({inline,yeccpars2_8_/1}).
-dialyzer({nowarn_function, yeccpars2_8_/1}).
-compile({nowarn_unused_function,  yeccpars2_8_/1}).
-file("src/absinthe_parser.yrl", 29).
yeccpars2_8_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                    ___1
  end | __Stack].

-compile({inline,yeccpars2_9_/1}).
-dialyzer({nowarn_function, yeccpars2_9_/1}).
-compile({nowarn_unused_function,  yeccpars2_9_/1}).
-file("src/absinthe_parser.yrl", 207).
yeccpars2_9_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                         ___1
  end | __Stack].

-compile({inline,yeccpars2_10_/1}).
-dialyzer({nowarn_function, yeccpars2_10_/1}).
-compile({nowarn_unused_function,  yeccpars2_10_/1}).
-file("src/absinthe_parser.yrl", 208).
yeccpars2_10_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                            ___1
  end | __Stack].

-compile({inline,yeccpars2_11_/1}).
-dialyzer({nowarn_function, yeccpars2_11_/1}).
-compile({nowarn_unused_function,  yeccpars2_11_/1}).
-file("src/absinthe_parser.yrl", 212).
yeccpars2_11_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                              ___1
  end | __Stack].

-compile({inline,yeccpars2_12_/1}).
-dialyzer({nowarn_function, yeccpars2_12_/1}).
-compile({nowarn_unused_function,  yeccpars2_12_/1}).
-file("src/absinthe_parser.yrl", 31).
yeccpars2_12_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                         ___1
  end | __Stack].

-compile({inline,yeccpars2_13_/1}).
-dialyzer({nowarn_function, yeccpars2_13_/1}).
-compile({nowarn_unused_function,  yeccpars2_13_/1}).
-file("src/absinthe_parser.yrl", 211).
yeccpars2_13_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                       ___1
  end | __Stack].

-compile({inline,yeccpars2_15_/1}).
-dialyzer({nowarn_function, yeccpars2_15_/1}).
-compile({nowarn_unused_function,  yeccpars2_15_/1}).
-file("src/absinthe_parser.yrl", 214).
yeccpars2_15_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                        ___1
  end | __Stack].

-compile({inline,yeccpars2_17_/1}).
-dialyzer({nowarn_function, yeccpars2_17_/1}).
-compile({nowarn_unused_function,  yeccpars2_17_/1}).
-file("src/absinthe_parser.yrl", 24).
yeccpars2_17_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                          build_ast_node('Document', #{'definitions' => ___1}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_18_/1}).
-dialyzer({nowarn_function, yeccpars2_18_/1}).
-compile({nowarn_unused_function,  yeccpars2_18_/1}).
-file("src/absinthe_parser.yrl", 26).
yeccpars2_18_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                            [___1]
  end | __Stack].

-compile({inline,yeccpars2_19_/1}).
-dialyzer({nowarn_function, yeccpars2_19_/1}).
-compile({nowarn_unused_function,  yeccpars2_19_/1}).
-file("src/absinthe_parser.yrl", 204).
yeccpars2_19_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                              extract_quoted_block_string_token(___1)
  end | __Stack].

-compile({inline,yeccpars2_26_/1}).
-dialyzer({nowarn_function, yeccpars2_26_/1}).
-compile({nowarn_unused_function,  yeccpars2_26_/1}).
-file("src/absinthe_parser.yrl", 36).
yeccpars2_26_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              ___1
  end | __Stack].

-compile({inline,yeccpars2_27_/1}).
-dialyzer({nowarn_function, yeccpars2_27_/1}).
-compile({nowarn_unused_function,  yeccpars2_27_/1}).
-file("src/absinthe_parser.yrl", 35).
yeccpars2_27_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                           ___1
  end | __Stack].

-compile({inline,yeccpars2_29_/1}).
-dialyzer({nowarn_function, yeccpars2_29_/1}).
-compile({nowarn_unused_function,  yeccpars2_29_/1}).
-file("src/absinthe_parser.yrl", 247).
yeccpars2_29_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                               build_ast_node('SchemaDeclaration', #{}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_30_/1}).
-dialyzer({nowarn_function, yeccpars2_30_/1}).
-compile({nowarn_unused_function,  yeccpars2_30_/1}).
-file("src/absinthe_parser.yrl", 203).
yeccpars2_30_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                        extract_quoted_string_token(___1)
  end | __Stack].

-compile({inline,yeccpars2_31_/1}).
-dialyzer({nowarn_function, yeccpars2_31_/1}).
-compile({nowarn_unused_function,  yeccpars2_31_/1}).
-file("src/absinthe_parser.yrl", 37).
yeccpars2_31_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                  ___1
  end | __Stack].

-compile({inline,yeccpars2_36_/1}).
-dialyzer({nowarn_function, yeccpars2_36_/1}).
-compile({nowarn_unused_function,  yeccpars2_36_/1}).
-file("src/absinthe_parser.yrl", 76).
yeccpars2_36_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                          [___1]
  end | __Stack].

-compile({inline,yeccpars2_37_/1}).
-dialyzer({nowarn_function, yeccpars2_37_/1}).
-compile({nowarn_unused_function,  yeccpars2_37_/1}).
-file("src/absinthe_parser.yrl", 154).
yeccpars2_37_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                        ___1
  end | __Stack].

-compile({inline,yeccpars2_38_/1}).
-dialyzer({nowarn_function, yeccpars2_38_/1}).
-compile({nowarn_unused_function,  yeccpars2_38_/1}).
-file("src/absinthe_parser.yrl", 93).
yeccpars2_38_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                build_ast_node('Field', #{'name' => extract_binary(___1)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_39_/1}).
-dialyzer({nowarn_function, yeccpars2_39_/1}).
-compile({nowarn_unused_function,  yeccpars2_39_/1}).
-file("src/absinthe_parser.yrl", 81).
yeccpars2_39_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              ___1
  end | __Stack].

-compile({inline,yeccpars2_40_/1}).
-dialyzer({nowarn_function, yeccpars2_40_/1}).
-compile({nowarn_unused_function,  yeccpars2_40_/1}).
-file("src/absinthe_parser.yrl", 80).
yeccpars2_40_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              ___1
  end | __Stack].

-compile({inline,yeccpars2_41_/1}).
-dialyzer({nowarn_function, yeccpars2_41_/1}).
-compile({nowarn_unused_function,  yeccpars2_41_/1}).
-file("src/absinthe_parser.yrl", 79).
yeccpars2_41_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                     ___1
  end | __Stack].

-compile({inline,yeccpars2_44_/1}).
-dialyzer({nowarn_function, yeccpars2_44_/1}).
-compile({nowarn_unused_function,  yeccpars2_44_/1}).
-file("src/absinthe_parser.yrl", 152).
yeccpars2_44_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                               ___1
  end | __Stack].

-compile({inline,yeccpars2_45_/1}).
-dialyzer({nowarn_function, yeccpars2_45_/1}).
-compile({nowarn_unused_function,  yeccpars2_45_/1}).
-file("src/absinthe_parser.yrl", 149).
yeccpars2_45_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                          ___1
  end | __Stack].

-compile({inline,yeccpars2_46_/1}).
-dialyzer({nowarn_function, yeccpars2_46_/1}).
-compile({nowarn_unused_function,  yeccpars2_46_/1}).
-file("src/absinthe_parser.yrl", 151).
yeccpars2_46_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                            ___1
  end | __Stack].

-compile({inline,yeccpars2_47_/1}).
-dialyzer({nowarn_function, yeccpars2_47_/1}).
-compile({nowarn_unused_function,  yeccpars2_47_/1}).
-file("src/absinthe_parser.yrl", 142).
yeccpars2_47_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              ___1
  end | __Stack].

-compile({inline,yeccpars2_48_/1}).
-dialyzer({nowarn_function, yeccpars2_48_/1}).
-compile({nowarn_unused_function,  yeccpars2_48_/1}).
-file("src/absinthe_parser.yrl", 144).
yeccpars2_48_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                ___1
  end | __Stack].

-compile({inline,yeccpars2_49_/1}).
-dialyzer({nowarn_function, yeccpars2_49_/1}).
-compile({nowarn_unused_function,  yeccpars2_49_/1}).
-file("src/absinthe_parser.yrl", 150).
yeccpars2_49_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                           ___1
  end | __Stack].

-compile({inline,yeccpars2_50_/1}).
-dialyzer({nowarn_function, yeccpars2_50_/1}).
-compile({nowarn_unused_function,  yeccpars2_50_/1}).
-file("src/absinthe_parser.yrl", 145).
yeccpars2_50_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                               ___1
  end | __Stack].

-compile({inline,yeccpars2_51_/1}).
-dialyzer({nowarn_function, yeccpars2_51_/1}).
-compile({nowarn_unused_function,  yeccpars2_51_/1}).
-file("src/absinthe_parser.yrl", 140).
yeccpars2_51_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              ___1
  end | __Stack].

-compile({inline,yeccpars2_52_/1}).
-dialyzer({nowarn_function, yeccpars2_52_/1}).
-compile({nowarn_unused_function,  yeccpars2_52_/1}).
-file("src/absinthe_parser.yrl", 138).
yeccpars2_52_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                          ___1
  end | __Stack].

-compile({inline,yeccpars2_53_/1}).
-dialyzer({nowarn_function, yeccpars2_53_/1}).
-compile({nowarn_unused_function,  yeccpars2_53_/1}).
-file("src/absinthe_parser.yrl", 155).
yeccpars2_53_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
               extract_binary(___1)
  end | __Stack].

-compile({inline,yeccpars2_54_/1}).
-dialyzer({nowarn_function, yeccpars2_54_/1}).
-compile({nowarn_unused_function,  yeccpars2_54_/1}).
-file("src/absinthe_parser.yrl", 139).
yeccpars2_54_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                           ___1
  end | __Stack].

-compile({inline,yeccpars2_55_/1}).
-dialyzer({nowarn_function, yeccpars2_55_/1}).
-compile({nowarn_unused_function,  yeccpars2_55_/1}).
-file("src/absinthe_parser.yrl", 147).
yeccpars2_55_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                            ___1
  end | __Stack].

-compile({inline,yeccpars2_56_/1}).
-dialyzer({nowarn_function, yeccpars2_56_/1}).
-compile({nowarn_unused_function,  yeccpars2_56_/1}).
-file("src/absinthe_parser.yrl", 148).
yeccpars2_56_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                            ___1
  end | __Stack].

-compile({inline,yeccpars2_57_/1}).
-dialyzer({nowarn_function, yeccpars2_57_/1}).
-compile({nowarn_unused_function,  yeccpars2_57_/1}).
-file("src/absinthe_parser.yrl", 141).
yeccpars2_57_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                  ___1
  end | __Stack].

-compile({inline,yeccpars2_58_/1}).
-dialyzer({nowarn_function, yeccpars2_58_/1}).
-compile({nowarn_unused_function,  yeccpars2_58_/1}).
-file("src/absinthe_parser.yrl", 143).
yeccpars2_58_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                          ___1
  end | __Stack].

-compile({inline,yeccpars2_59_/1}).
-dialyzer({nowarn_function, yeccpars2_59_/1}).
-compile({nowarn_unused_function,  yeccpars2_59_/1}).
-file("src/absinthe_parser.yrl", 146).
yeccpars2_59_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                           ___1
  end | __Stack].

-compile({inline,yeccpars2_60_/1}).
-dialyzer({nowarn_function, yeccpars2_60_/1}).
-compile({nowarn_unused_function,  yeccpars2_60_/1}).
-file("src/absinthe_parser.yrl", 89).
yeccpars2_60_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                       build_ast_node('InlineFragment', #{'selection_set' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_61_/1}).
-dialyzer({nowarn_function, yeccpars2_61_/1}).
-compile({nowarn_unused_function,  yeccpars2_61_/1}).
-file("src/absinthe_parser.yrl", 91).
yeccpars2_61_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                extract_binary(___1)
  end | __Stack].

-compile({inline,yeccpars2_62_/1}).
-dialyzer({nowarn_function, yeccpars2_62_/1}).
-compile({nowarn_unused_function,  yeccpars2_62_/1}).
-file("src/absinthe_parser.yrl", 83).
yeccpars2_62_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                       build_ast_node('FragmentSpread', #{'name' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_64_/1}).
-dialyzer({nowarn_function, yeccpars2_64_/1}).
-compile({nowarn_unused_function,  yeccpars2_64_/1}).
-file("src/absinthe_parser.yrl", 131).
yeccpars2_64_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                          [___1]
  end | __Stack].

-compile({inline,yeccpars2_68_/1}).
-dialyzer({nowarn_function, yeccpars2_68_/1}).
-compile({nowarn_unused_function,  yeccpars2_68_/1}).
-file("src/absinthe_parser.yrl", 52).
yeccpars2_68_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                             ___1
  end | __Stack].

-compile({inline,yeccpars2_69_/1}).
-dialyzer({nowarn_function, yeccpars2_69_/1}).
-compile({nowarn_unused_function,  yeccpars2_69_/1}).
-file("src/absinthe_parser.yrl", 69).
yeccpars2_69_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                    build_ast_node('NamedType', #{'name' => extract_binary(___1)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_70_/1}).
-dialyzer({nowarn_function, yeccpars2_70_/1}).
-compile({nowarn_unused_function,  yeccpars2_70_/1}).
-file("src/absinthe_parser.yrl", 86).
yeccpars2_70_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                          build_ast_node('InlineFragment', #{'type_condition' => ___3, 'selection_set' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_72_/1}).
-dialyzer({nowarn_function, yeccpars2_72_/1}).
-compile({nowarn_unused_function,  yeccpars2_72_/1}).
-file("src/absinthe_parser.yrl", 87).
yeccpars2_72_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                     build_ast_node('InlineFragment', #{'type_condition' => ___3, 'directives' => ___4, 'selection_set' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_73_/1}).
-dialyzer({nowarn_function, yeccpars2_73_/1}).
-compile({nowarn_unused_function,  yeccpars2_73_/1}).
-file("src/absinthe_parser.yrl", 133).
yeccpars2_73_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                 build_ast_node('Directive', #{name => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_74_/1}).
-dialyzer({nowarn_function, yeccpars2_74_/1}).
-compile({nowarn_unused_function,  yeccpars2_74_/1}).
-file("src/absinthe_parser.yrl", 135).
yeccpars2_74_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                        build_ast_node('Directive', #{name => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_75_/1}).
-dialyzer({nowarn_function, yeccpars2_75_/1}).
-compile({nowarn_unused_function,  yeccpars2_75_/1}).
-file("src/absinthe_parser.yrl", 136).
yeccpars2_75_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                  build_ast_node('Directive', #{name => extract_binary(___2), 'arguments' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_79_/1}).
-dialyzer({nowarn_function, yeccpars2_79_/1}).
-compile({nowarn_unused_function,  yeccpars2_79_/1}).
-file("src/absinthe_parser.yrl", 113).
yeccpars2_79_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                           [___1]
  end | __Stack].

-compile({inline,yeccpars2_82_/1}).
-dialyzer({nowarn_function, yeccpars2_82_/1}).
-compile({nowarn_unused_function,  yeccpars2_82_/1}).
-file("src/absinthe_parser.yrl", 157).
yeccpars2_82_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                    ___1
  end | __Stack].

-compile({inline,yeccpars2_83_/1}).
-dialyzer({nowarn_function, yeccpars2_83_/1}).
-compile({nowarn_unused_function,  yeccpars2_83_/1}).
-file("src/absinthe_parser.yrl", 116).
yeccpars2_83_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                             build_ast_node('Argument', #{name => extract_binary(___1), value => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_84_/1}).
-dialyzer({nowarn_function, yeccpars2_84_/1}).
-compile({nowarn_unused_function,  yeccpars2_84_/1}).
-file("src/absinthe_parser.yrl", 166).
yeccpars2_84_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                         build_ast_node('ObjectValue',  #{'fields' => ___1}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_85_/1}).
-dialyzer({nowarn_function, yeccpars2_85_/1}).
-compile({nowarn_unused_function,  yeccpars2_85_/1}).
-file("src/absinthe_parser.yrl", 178).
yeccpars2_85_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                    extract_binary(___1)
  end | __Stack].

-compile({inline,yeccpars2_86_/1}).
-dialyzer({nowarn_function, yeccpars2_86_/1}).
-compile({nowarn_unused_function,  yeccpars2_86_/1}).
-file("src/absinthe_parser.yrl", 165).
yeccpars2_86_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                         build_ast_node('ListValue',    #{'values' => ___1}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_87_/1}).
-dialyzer({nowarn_function, yeccpars2_87_/1}).
-compile({nowarn_unused_function,  yeccpars2_87_/1}).
-file("src/absinthe_parser.yrl", 164).
yeccpars2_87_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                         build_ast_node('EnumValue',    #{'value' => ___1},  extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_90_/1}).
-dialyzer({nowarn_function, yeccpars2_90_/1}).
-compile({nowarn_unused_function,  yeccpars2_90_/1}).
-file("src/absinthe_parser.yrl", 160).
yeccpars2_90_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                               build_ast_node('StringValue',  #{'value' => extract_quoted_block_string_token(___1)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_91_/1}).
-dialyzer({nowarn_function, yeccpars2_91_/1}).
-compile({nowarn_unused_function,  yeccpars2_91_/1}).
-file("src/absinthe_parser.yrl", 162).
yeccpars2_91_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                         build_ast_node('BooleanValue', #{'value' => extract_boolean(___1)},             extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_92_/1}).
-dialyzer({nowarn_function, yeccpars2_92_/1}).
-compile({nowarn_unused_function,  yeccpars2_92_/1}).
-file("src/absinthe_parser.yrl", 159).
yeccpars2_92_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                         build_ast_node('FloatValue',   #{'value' => extract_float(___1)},               extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_93_/1}).
-dialyzer({nowarn_function, yeccpars2_93_/1}).
-compile({nowarn_unused_function,  yeccpars2_93_/1}).
-file("src/absinthe_parser.yrl", 158).
yeccpars2_93_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                         build_ast_node('IntValue',     #{'value' => extract_integer(___1)},             extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_94_/1}).
-dialyzer({nowarn_function, yeccpars2_94_/1}).
-compile({nowarn_unused_function,  yeccpars2_94_/1}).
-file("src/absinthe_parser.yrl", 163).
yeccpars2_94_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                         build_ast_node('NullValue',    #{},                 extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_95_/1}).
-dialyzer({nowarn_function, yeccpars2_95_/1}).
-compile({nowarn_unused_function,  yeccpars2_95_/1}).
-file("src/absinthe_parser.yrl", 161).
yeccpars2_95_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                         build_ast_node('StringValue',  #{'value' => extract_quoted_string_token(___1)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_98_/1}).
-dialyzer({nowarn_function, yeccpars2_98_/1}).
-compile({nowarn_unused_function,  yeccpars2_98_/1}).
-file("src/absinthe_parser.yrl", 199).
yeccpars2_98_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              [___1]
  end | __Stack].

-compile({inline,yeccpars2_100_/1}).
-dialyzer({nowarn_function, yeccpars2_100_/1}).
-compile({nowarn_unused_function,  yeccpars2_100_/1}).
-file("src/absinthe_parser.yrl", 197).
yeccpars2_100_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                         []
  end | __Stack].

-compile({inline,yeccpars2_102_/1}).
-dialyzer({nowarn_function, yeccpars2_102_/1}).
-compile({nowarn_unused_function,  yeccpars2_102_/1}).
-file("src/absinthe_parser.yrl", 201).
yeccpars2_102_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                build_ast_node('ObjectField', #{'name' => extract_binary(___1), 'value' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_103_/1}).
-dialyzer({nowarn_function, yeccpars2_103_/1}).
-compile({nowarn_unused_function,  yeccpars2_103_/1}).
-file("src/absinthe_parser.yrl", 200).
yeccpars2_103_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                           [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_104_/1}).
-dialyzer({nowarn_function, yeccpars2_104_/1}).
-compile({nowarn_unused_function,  yeccpars2_104_/1}).
-file("src/absinthe_parser.yrl", 198).
yeccpars2_104_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                      ___2
  end | __Stack].

-compile({inline,yeccpars2_106_/1}).
-dialyzer({nowarn_function, yeccpars2_106_/1}).
-compile({nowarn_unused_function,  yeccpars2_106_/1}).
-file("src/absinthe_parser.yrl", 187).
yeccpars2_106_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                  [___1]
  end | __Stack].

-compile({inline,yeccpars2_107_/1}).
-dialyzer({nowarn_function, yeccpars2_107_/1}).
-compile({nowarn_unused_function,  yeccpars2_107_/1}).
-file("src/absinthe_parser.yrl", 185).
yeccpars2_107_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                       []
  end | __Stack].

-compile({inline,yeccpars2_108_/1}).
-dialyzer({nowarn_function, yeccpars2_108_/1}).
-compile({nowarn_unused_function,  yeccpars2_108_/1}).
-file("src/absinthe_parser.yrl", 188).
yeccpars2_108_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                         [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_109_/1}).
-dialyzer({nowarn_function, yeccpars2_109_/1}).
-compile({nowarn_unused_function,  yeccpars2_109_/1}).
-file("src/absinthe_parser.yrl", 186).
yeccpars2_109_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                              ___2
  end | __Stack].

-compile({inline,yeccpars2_110_/1}).
-dialyzer({nowarn_function, yeccpars2_110_/1}).
-compile({nowarn_unused_function,  yeccpars2_110_/1}).
-file("src/absinthe_parser.yrl", 61).
yeccpars2_110_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                build_ast_node('Variable', #{'name' => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_111_/1}).
-dialyzer({nowarn_function, yeccpars2_111_/1}).
-compile({nowarn_unused_function,  yeccpars2_111_/1}).
-file("src/absinthe_parser.yrl", 62).
yeccpars2_111_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                       build_ast_node('Variable', #{'name' => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_112_/1}).
-dialyzer({nowarn_function, yeccpars2_112_/1}).
-compile({nowarn_unused_function,  yeccpars2_112_/1}).
-file("src/absinthe_parser.yrl", 114).
yeccpars2_112_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                        [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_113_/1}).
-dialyzer({nowarn_function, yeccpars2_113_/1}).
-compile({nowarn_unused_function,  yeccpars2_113_/1}).
-file("src/absinthe_parser.yrl", 112).
yeccpars2_113_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                    ___2
  end | __Stack].

-compile({inline,yeccpars2_115_/1}).
-dialyzer({nowarn_function, yeccpars2_115_/1}).
-compile({nowarn_unused_function,  yeccpars2_115_/1}).
-file("src/absinthe_parser.yrl", 115).
yeccpars2_115_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                      build_ast_node('Argument', #{name => extract_binary(___1), value => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_116_/1}).
-dialyzer({nowarn_function, yeccpars2_116_/1}).
-compile({nowarn_unused_function,  yeccpars2_116_/1}).
-file("src/absinthe_parser.yrl", 134).
yeccpars2_116_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                           build_ast_node('Directive', #{name => extract_binary(___2), 'arguments' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_117_/1}).
-dialyzer({nowarn_function, yeccpars2_117_/1}).
-compile({nowarn_unused_function,  yeccpars2_117_/1}).
-file("src/absinthe_parser.yrl", 132).
yeccpars2_117_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                     [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_118_/1}).
-dialyzer({nowarn_function, yeccpars2_118_/1}).
-compile({nowarn_unused_function,  yeccpars2_118_/1}).
-file("src/absinthe_parser.yrl", 88).
yeccpars2_118_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                  build_ast_node('InlineFragment', #{'directives' => ___2, 'selection_set' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_119_/1}).
-dialyzer({nowarn_function, yeccpars2_119_/1}).
-compile({nowarn_unused_function,  yeccpars2_119_/1}).
-file("src/absinthe_parser.yrl", 84).
yeccpars2_119_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                  build_ast_node('FragmentSpread', #{'name' => ___2, 'directives' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_120_/1}).
-dialyzer({nowarn_function, yeccpars2_120_/1}).
-compile({nowarn_unused_function,  yeccpars2_120_/1}).
-file("src/absinthe_parser.yrl", 101).
yeccpars2_120_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                      build_ast_node('Field', #{'alias' => extract_binary(___1), 'name' => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_121_/1}).
-dialyzer({nowarn_function, yeccpars2_121_/1}).
-compile({nowarn_unused_function,  yeccpars2_121_/1}).
-file("src/absinthe_parser.yrl", 103).
yeccpars2_121_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                   build_ast_node('Field', #{'alias' => extract_binary(___1), 'name' => extract_binary(___2), 'selection_set' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_122_/1}).
-dialyzer({nowarn_function, yeccpars2_122_/1}).
-compile({nowarn_unused_function,  yeccpars2_122_/1}).
-file("src/absinthe_parser.yrl", 105).
yeccpars2_122_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                 build_ast_node('Field', #{'alias' => extract_binary(___1), 'name' => extract_binary(___2), 'directives' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_123_/1}).
-dialyzer({nowarn_function, yeccpars2_123_/1}).
-compile({nowarn_unused_function,  yeccpars2_123_/1}).
-file("src/absinthe_parser.yrl", 102).
yeccpars2_123_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                build_ast_node('Field', #{'alias' => extract_binary(___1), 'name' => extract_binary(___2), 'arguments' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_124_/1}).
-dialyzer({nowarn_function, yeccpars2_124_/1}).
-compile({nowarn_unused_function,  yeccpars2_124_/1}).
-file("src/absinthe_parser.yrl", 104).
yeccpars2_124_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                             build_ast_node('Field', #{'alias' => extract_binary(___1), 'name' => extract_binary(___2), 'arguments' => ___3, 'selection_set' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_125_/1}).
-dialyzer({nowarn_function, yeccpars2_125_/1}).
-compile({nowarn_unused_function,  yeccpars2_125_/1}).
-file("src/absinthe_parser.yrl", 106).
yeccpars2_125_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                           build_ast_node('Field', #{'alias' => extract_binary(___1), 'name' => extract_binary(___2), 'arguments' => ___3, 'directives' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_126_/1}).
-dialyzer({nowarn_function, yeccpars2_126_/1}).
-compile({nowarn_unused_function,  yeccpars2_126_/1}).
-file("src/absinthe_parser.yrl", 108).
yeccpars2_126_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                        build_ast_node('Field', #{'alias' => extract_binary(___1), 'name' => extract_binary(___2), 'arguments' => ___3, 'directives' => ___4, 'selection_set' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_127_/1}).
-dialyzer({nowarn_function, yeccpars2_127_/1}).
-compile({nowarn_unused_function,  yeccpars2_127_/1}).
-file("src/absinthe_parser.yrl", 107).
yeccpars2_127_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                              build_ast_node('Field', #{'alias' => extract_binary(___1), 'name' => extract_binary(___2), 'directives' => ___3, 'selection_set' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_128_/1}).
-dialyzer({nowarn_function, yeccpars2_128_/1}).
-compile({nowarn_unused_function,  yeccpars2_128_/1}).
-file("src/absinthe_parser.yrl", 96).
yeccpars2_128_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                             build_ast_node('Field', #{'name' => extract_binary(___1), 'selection_set' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_129_/1}).
-dialyzer({nowarn_function, yeccpars2_129_/1}).
-compile({nowarn_unused_function,  yeccpars2_129_/1}).
-file("src/absinthe_parser.yrl", 95).
yeccpars2_129_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                           build_ast_node('Field', #{'name' => extract_binary(___1), 'directives' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_130_/1}).
-dialyzer({nowarn_function, yeccpars2_130_/1}).
-compile({nowarn_unused_function,  yeccpars2_130_/1}).
-file("src/absinthe_parser.yrl", 94).
yeccpars2_130_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                          build_ast_node('Field', #{'name' => extract_binary(___1), 'arguments' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_131_/1}).
-dialyzer({nowarn_function, yeccpars2_131_/1}).
-compile({nowarn_unused_function,  yeccpars2_131_/1}).
-file("src/absinthe_parser.yrl", 110).
yeccpars2_131_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                    ___1
  end | __Stack].

-compile({inline,yeccpars2_132_/1}).
-dialyzer({nowarn_function, yeccpars2_132_/1}).
-compile({nowarn_unused_function,  yeccpars2_132_/1}).
-file("src/absinthe_parser.yrl", 98).
yeccpars2_132_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                       build_ast_node('Field', #{'name' => extract_binary(___1), 'arguments' => ___2, 'selection_set' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_133_/1}).
-dialyzer({nowarn_function, yeccpars2_133_/1}).
-compile({nowarn_unused_function,  yeccpars2_133_/1}).
-file("src/absinthe_parser.yrl", 99).
yeccpars2_133_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                     build_ast_node('Field', #{'name' => extract_binary(___1), 'arguments' => ___2, 'directives' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_134_/1}).
-dialyzer({nowarn_function, yeccpars2_134_/1}).
-compile({nowarn_unused_function,  yeccpars2_134_/1}).
-file("src/absinthe_parser.yrl", 100).
yeccpars2_134_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                  build_ast_node('Field', #{'name' => extract_binary(___1), 'arguments' => ___2, 'directives' => ___3, 'selection_set' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_135_/1}).
-dialyzer({nowarn_function, yeccpars2_135_/1}).
-compile({nowarn_unused_function,  yeccpars2_135_/1}).
-file("src/absinthe_parser.yrl", 97).
yeccpars2_135_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                        build_ast_node('Field', #{'name' => extract_binary(___1), 'directives' => ___2, 'selection_set' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_136_/1}).
-dialyzer({nowarn_function, yeccpars2_136_/1}).
-compile({nowarn_unused_function,  yeccpars2_136_/1}).
-file("src/absinthe_parser.yrl", 77).
yeccpars2_136_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                     [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_137_/1}).
-dialyzer({nowarn_function, yeccpars2_137_/1}).
-compile({nowarn_unused_function,  yeccpars2_137_/1}).
-file("src/absinthe_parser.yrl", 74).
yeccpars2_137_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                     build_ast_node('SelectionSet', #{'selections' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_138_/1}).
-dialyzer({nowarn_function, yeccpars2_138_/1}).
-compile({nowarn_unused_function,  yeccpars2_138_/1}).
-file("src/absinthe_parser.yrl", 308).
yeccpars2_138_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                     
  build_ast_node('UnionTypeDefinition', #{'name' => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_139_/1}).
-dialyzer({nowarn_function, yeccpars2_139_/1}).
-compile({nowarn_unused_function,  yeccpars2_139_/1}).
-file("src/absinthe_parser.yrl", 310).
yeccpars2_139_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                     
  build_ast_node('UnionTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_140_/1}).
-dialyzer({nowarn_function, yeccpars2_140_/1}).
-compile({nowarn_unused_function,  yeccpars2_140_/1}).
-file("src/absinthe_parser.yrl", 118).
yeccpars2_140_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                    [___1]
  end | __Stack].

-compile({inline,yeccpars2_143_/1}).
-dialyzer({nowarn_function, yeccpars2_143_/1}).
-compile({nowarn_unused_function,  yeccpars2_143_/1}).
-file("src/absinthe_parser.yrl", 120).
yeccpars2_143_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                      build_ast_node('Directive', #{name => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_144_/1}).
-dialyzer({nowarn_function, yeccpars2_144_/1}).
-compile({nowarn_unused_function,  yeccpars2_144_/1}).
-file("src/absinthe_parser.yrl", 122).
yeccpars2_144_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                             build_ast_node('Directive', #{name => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_145_/1}).
-dialyzer({nowarn_function, yeccpars2_145_/1}).
-compile({nowarn_unused_function,  yeccpars2_145_/1}).
-file("src/absinthe_parser.yrl", 123).
yeccpars2_145_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                            build_ast_node('Directive', #{name => extract_binary(___2), 'arguments' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_149_/1}).
-dialyzer({nowarn_function, yeccpars2_149_/1}).
-compile({nowarn_unused_function,  yeccpars2_149_/1}).
-file("src/absinthe_parser.yrl", 126).
yeccpars2_149_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                     [___1]
  end | __Stack].

-compile({inline,yeccpars2_152_/1}).
-dialyzer({nowarn_function, yeccpars2_152_/1}).
-compile({nowarn_unused_function,  yeccpars2_152_/1}).
-file("src/absinthe_parser.yrl", 129).
yeccpars2_152_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                       build_ast_node('Argument', #{name => extract_binary(___1), value => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_153_/1}).
-dialyzer({nowarn_function, yeccpars2_153_/1}).
-compile({nowarn_unused_function,  yeccpars2_153_/1}).
-file("src/absinthe_parser.yrl", 176).
yeccpars2_153_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                   build_ast_node('ObjectValue',  #{'fields' => ___1}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_154_/1}).
-dialyzer({nowarn_function, yeccpars2_154_/1}).
-compile({nowarn_unused_function,  yeccpars2_154_/1}).
-file("src/absinthe_parser.yrl", 175).
yeccpars2_154_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                   build_ast_node('ListValue',    #{'values' => ___1}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_155_/1}).
-dialyzer({nowarn_function, yeccpars2_155_/1}).
-compile({nowarn_unused_function,  yeccpars2_155_/1}).
-file("src/absinthe_parser.yrl", 174).
yeccpars2_155_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              build_ast_node('EnumValue',    #{'value' => ___1},  extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_157_/1}).
-dialyzer({nowarn_function, yeccpars2_157_/1}).
-compile({nowarn_unused_function,  yeccpars2_157_/1}).
-file("src/absinthe_parser.yrl", 170).
yeccpars2_157_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                    build_ast_node('StringValue',  #{'value' => extract_quoted_block_string_token(___1)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_158_/1}).
-dialyzer({nowarn_function, yeccpars2_158_/1}).
-compile({nowarn_unused_function,  yeccpars2_158_/1}).
-file("src/absinthe_parser.yrl", 172).
yeccpars2_158_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              build_ast_node('BooleanValue', #{'value' => extract_boolean(___1)},             extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_159_/1}).
-dialyzer({nowarn_function, yeccpars2_159_/1}).
-compile({nowarn_unused_function,  yeccpars2_159_/1}).
-file("src/absinthe_parser.yrl", 169).
yeccpars2_159_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              build_ast_node('FloatValue',   #{'value' => extract_float(___1)},               extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_160_/1}).
-dialyzer({nowarn_function, yeccpars2_160_/1}).
-compile({nowarn_unused_function,  yeccpars2_160_/1}).
-file("src/absinthe_parser.yrl", 168).
yeccpars2_160_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              build_ast_node('IntValue',     #{'value' => extract_integer(___1)},             extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_161_/1}).
-dialyzer({nowarn_function, yeccpars2_161_/1}).
-compile({nowarn_unused_function,  yeccpars2_161_/1}).
-file("src/absinthe_parser.yrl", 173).
yeccpars2_161_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              build_ast_node('NullValue',    #{},                 extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_162_/1}).
-dialyzer({nowarn_function, yeccpars2_162_/1}).
-compile({nowarn_unused_function,  yeccpars2_162_/1}).
-file("src/absinthe_parser.yrl", 171).
yeccpars2_162_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                              build_ast_node('StringValue',  #{'value' => extract_quoted_string_token(___1)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_165_/1}).
-dialyzer({nowarn_function, yeccpars2_165_/1}).
-compile({nowarn_unused_function,  yeccpars2_165_/1}).
-file("src/absinthe_parser.yrl", 192).
yeccpars2_165_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                        [___1]
  end | __Stack].

-compile({inline,yeccpars2_167_/1}).
-dialyzer({nowarn_function, yeccpars2_167_/1}).
-compile({nowarn_unused_function,  yeccpars2_167_/1}).
-file("src/absinthe_parser.yrl", 190).
yeccpars2_167_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                              []
  end | __Stack].

-compile({inline,yeccpars2_169_/1}).
-dialyzer({nowarn_function, yeccpars2_169_/1}).
-compile({nowarn_unused_function,  yeccpars2_169_/1}).
-file("src/absinthe_parser.yrl", 194).
yeccpars2_169_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                          build_ast_node('ObjectField', #{'name' => extract_binary(___1), 'value' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_170_/1}).
-dialyzer({nowarn_function, yeccpars2_170_/1}).
-compile({nowarn_unused_function,  yeccpars2_170_/1}).
-file("src/absinthe_parser.yrl", 193).
yeccpars2_170_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                          [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_171_/1}).
-dialyzer({nowarn_function, yeccpars2_171_/1}).
-compile({nowarn_unused_function,  yeccpars2_171_/1}).
-file("src/absinthe_parser.yrl", 191).
yeccpars2_171_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                ___2
  end | __Stack].

-compile({inline,yeccpars2_173_/1}).
-dialyzer({nowarn_function, yeccpars2_173_/1}).
-compile({nowarn_unused_function,  yeccpars2_173_/1}).
-file("src/absinthe_parser.yrl", 182).
yeccpars2_173_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                            [___1]
  end | __Stack].

-compile({inline,yeccpars2_174_/1}).
-dialyzer({nowarn_function, yeccpars2_174_/1}).
-compile({nowarn_unused_function,  yeccpars2_174_/1}).
-file("src/absinthe_parser.yrl", 180).
yeccpars2_174_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                            []
  end | __Stack].

-compile({inline,yeccpars2_175_/1}).
-dialyzer({nowarn_function, yeccpars2_175_/1}).
-compile({nowarn_unused_function,  yeccpars2_175_/1}).
-file("src/absinthe_parser.yrl", 183).
yeccpars2_175_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                        [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_176_/1}).
-dialyzer({nowarn_function, yeccpars2_176_/1}).
-compile({nowarn_unused_function,  yeccpars2_176_/1}).
-file("src/absinthe_parser.yrl", 181).
yeccpars2_176_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                        ___2
  end | __Stack].

-compile({inline,yeccpars2_177_/1}).
-dialyzer({nowarn_function, yeccpars2_177_/1}).
-compile({nowarn_unused_function,  yeccpars2_177_/1}).
-file("src/absinthe_parser.yrl", 127).
yeccpars2_177_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                       [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_178_/1}).
-dialyzer({nowarn_function, yeccpars2_178_/1}).
-compile({nowarn_unused_function,  yeccpars2_178_/1}).
-file("src/absinthe_parser.yrl", 125).
yeccpars2_178_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                              ___2
  end | __Stack].

-compile({inline,yeccpars2_180_/1}).
-dialyzer({nowarn_function, yeccpars2_180_/1}).
-compile({nowarn_unused_function,  yeccpars2_180_/1}).
-file("src/absinthe_parser.yrl", 128).
yeccpars2_180_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                build_ast_node('Argument', #{name => extract_binary(___1), value => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_181_/1}).
-dialyzer({nowarn_function, yeccpars2_181_/1}).
-compile({nowarn_unused_function,  yeccpars2_181_/1}).
-file("src/absinthe_parser.yrl", 121).
yeccpars2_181_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                     build_ast_node('Directive', #{name => extract_binary(___2), 'arguments' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_182_/1}).
-dialyzer({nowarn_function, yeccpars2_182_/1}).
-compile({nowarn_unused_function,  yeccpars2_182_/1}).
-file("src/absinthe_parser.yrl", 312).
yeccpars2_182_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                      
  build_ast_node('UnionTypeDefinition', #{'name' => extract_binary(___2), 'types' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_183_/1}).
-dialyzer({nowarn_function, yeccpars2_183_/1}).
-compile({nowarn_unused_function,  yeccpars2_183_/1}).
-file("src/absinthe_parser.yrl", 317).
yeccpars2_183_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                            [___1]
  end | __Stack].

-compile({inline,yeccpars2_187_/1}).
-dialyzer({nowarn_function, yeccpars2_187_/1}).
-compile({nowarn_unused_function,  yeccpars2_187_/1}).
-file("src/absinthe_parser.yrl", 319).
yeccpars2_187_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                 [___2|___4]
  end | __Stack].

-compile({inline,yeccpars2_189_/1}).
-dialyzer({nowarn_function, yeccpars2_189_/1}).
-compile({nowarn_unused_function,  yeccpars2_189_/1}).
-file("src/absinthe_parser.yrl", 318).
yeccpars2_189_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                             [___1|___3]
  end | __Stack].

-compile({inline,yeccpars2_190_/1}).
-dialyzer({nowarn_function, yeccpars2_190_/1}).
-compile({nowarn_unused_function,  yeccpars2_190_/1}).
-file("src/absinthe_parser.yrl", 119).
yeccpars2_190_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                    [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_192_/1}).
-dialyzer({nowarn_function, yeccpars2_192_/1}).
-compile({nowarn_unused_function,  yeccpars2_192_/1}).
-file("src/absinthe_parser.yrl", 314).
yeccpars2_192_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                      
  build_ast_node('UnionTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3, 'types' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_193_/1}).
-dialyzer({nowarn_function, yeccpars2_193_/1}).
-compile({nowarn_unused_function,  yeccpars2_193_/1}).
-file("src/absinthe_parser.yrl", 252).
yeccpars2_193_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                     
  build_ast_node('ObjectTypeDefinition', #{'name' => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_195_/1}).
-dialyzer({nowarn_function, yeccpars2_195_/1}).
-compile({nowarn_unused_function,  yeccpars2_195_/1}).
-file("src/absinthe_parser.yrl", 254).
yeccpars2_195_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                     
  build_ast_node('ObjectTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_200_/1}).
-dialyzer({nowarn_function, yeccpars2_200_/1}).
-compile({nowarn_unused_function,  yeccpars2_200_/1}).
-file("src/absinthe_parser.yrl", 270).
yeccpars2_200_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                         [___1]
  end | __Stack].

-compile({inline,yeccpars2_202_/1}).
-dialyzer({nowarn_function, yeccpars2_202_/1}).
-compile({nowarn_unused_function,  yeccpars2_202_/1}).
-file("src/absinthe_parser.yrl", 272).
yeccpars2_202_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                               [put_description(___2, ___1)]
  end | __Stack].

-compile({inline,yeccpars2_203_/1}).
-dialyzer({nowarn_function, yeccpars2_203_/1}).
-compile({nowarn_unused_function,  yeccpars2_203_/1}).
-file("src/absinthe_parser.yrl", 273).
yeccpars2_203_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                   [put_description(___2, ___1)|___3]
  end | __Stack].

-compile({inline,yeccpars2_204_/1}).
-dialyzer({nowarn_function, yeccpars2_204_/1}).
-compile({nowarn_unused_function,  yeccpars2_204_/1}).
-file("src/absinthe_parser.yrl", 271).
yeccpars2_204_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                             [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_205_/1}).
-dialyzer({nowarn_function, yeccpars2_205_/1}).
-compile({nowarn_unused_function,  yeccpars2_205_/1}).
-file("src/absinthe_parser.yrl", 256).
yeccpars2_205_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                 
  build_ast_node('ObjectTypeDefinition', #{'name' => extract_binary(___2), 'fields' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_210_/1}).
-dialyzer({nowarn_function, yeccpars2_210_/1}).
-compile({nowarn_unused_function,  yeccpars2_210_/1}).
-file("src/absinthe_parser.yrl", 275).
yeccpars2_210_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                   build_ast_node('FieldDefinition', #{'name' => extract_binary(___1), 'type' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_211_/1}).
-dialyzer({nowarn_function, yeccpars2_211_/1}).
-compile({nowarn_unused_function,  yeccpars2_211_/1}).
-file("src/absinthe_parser.yrl", 68).
yeccpars2_211_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                      ___1
  end | __Stack].

-compile({inline,yeccpars2_212_/1}).
-dialyzer({nowarn_function, yeccpars2_212_/1}).
-compile({nowarn_unused_function,  yeccpars2_212_/1}).
-file("src/absinthe_parser.yrl", 66).
yeccpars2_212_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                    ___1
  end | __Stack].

-compile({inline,yeccpars2_213_/1}).
-dialyzer({nowarn_function, yeccpars2_213_/1}).
-compile({nowarn_unused_function,  yeccpars2_213_/1}).
-file("src/absinthe_parser.yrl", 67).
yeccpars2_213_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                   ___1
  end | __Stack].

-compile({inline,yeccpars2_216_/1}).
-dialyzer({nowarn_function, yeccpars2_216_/1}).
-compile({nowarn_unused_function,  yeccpars2_216_/1}).
-file("src/absinthe_parser.yrl", 70).
yeccpars2_216_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                           build_ast_node('ListType', #{'type' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_217_/1}).
-dialyzer({nowarn_function, yeccpars2_217_/1}).
-compile({nowarn_unused_function,  yeccpars2_217_/1}).
-file("src/absinthe_parser.yrl", 72).
yeccpars2_217_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                              build_ast_node('NonNullType', #{'type' => ___1}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_218_/1}).
-dialyzer({nowarn_function, yeccpars2_218_/1}).
-compile({nowarn_unused_function,  yeccpars2_218_/1}).
-file("src/absinthe_parser.yrl", 71).
yeccpars2_218_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                               build_ast_node('NonNullType', #{'type' => ___1}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_219_/1}).
-dialyzer({nowarn_function, yeccpars2_219_/1}).
-compile({nowarn_unused_function,  yeccpars2_219_/1}).
-file("src/absinthe_parser.yrl", 276).
yeccpars2_219_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                   build_ast_node('FieldDefinition', #{'name' => extract_binary(___1), 'type' => ___3, 'directives' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_222_/1}).
-dialyzer({nowarn_function, yeccpars2_222_/1}).
-compile({nowarn_unused_function,  yeccpars2_222_/1}).
-file("src/absinthe_parser.yrl", 283).
yeccpars2_222_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                                   [___1]
  end | __Stack].

-compile({inline,yeccpars2_224_/1}).
-dialyzer({nowarn_function, yeccpars2_224_/1}).
-compile({nowarn_unused_function,  yeccpars2_224_/1}).
-file("src/absinthe_parser.yrl", 286).
yeccpars2_224_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                                         [put_description(___2, ___1)]
  end | __Stack].

-compile({inline,yeccpars2_225_/1}).
-dialyzer({nowarn_function, yeccpars2_225_/1}).
-compile({nowarn_unused_function,  yeccpars2_225_/1}).
-file("src/absinthe_parser.yrl", 287).
yeccpars2_225_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                                  [put_description(___2, ___1)|___3]
  end | __Stack].

-compile({inline,yeccpars2_226_/1}).
-dialyzer({nowarn_function, yeccpars2_226_/1}).
-compile({nowarn_unused_function,  yeccpars2_226_/1}).
-file("src/absinthe_parser.yrl", 284).
yeccpars2_226_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                                            [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_227_/1}).
-dialyzer({nowarn_function, yeccpars2_227_/1}).
-compile({nowarn_unused_function,  yeccpars2_227_/1}).
-file("src/absinthe_parser.yrl", 281).
yeccpars2_227_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                          ___2
  end | __Stack].

-compile({inline,yeccpars2_229_/1}).
-dialyzer({nowarn_function, yeccpars2_229_/1}).
-compile({nowarn_unused_function,  yeccpars2_229_/1}).
-file("src/absinthe_parser.yrl", 289).
yeccpars2_229_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                        build_ast_node('InputValueDefinition', #{'name' => extract_binary(___1), 'type' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_230_/1}).
-dialyzer({nowarn_function, yeccpars2_230_/1}).
-compile({nowarn_unused_function,  yeccpars2_230_/1}).
-file("src/absinthe_parser.yrl", 290).
yeccpars2_230_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                        build_ast_node('InputValueDefinition', #{'name' => extract_binary(___1), 'type' => ___3, 'directives' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_231_/1}).
-dialyzer({nowarn_function, yeccpars2_231_/1}).
-compile({nowarn_unused_function,  yeccpars2_231_/1}).
-file("src/absinthe_parser.yrl", 291).
yeccpars2_231_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                     build_ast_node('InputValueDefinition', #{'name' => extract_binary(___1), 'type' => ___3, 'default_value' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_233_/1}).
-dialyzer({nowarn_function, yeccpars2_233_/1}).
-compile({nowarn_unused_function,  yeccpars2_233_/1}).
-file("src/absinthe_parser.yrl", 64).
yeccpars2_233_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                 ___2
  end | __Stack].

-compile({inline,yeccpars2_234_/1}).
-dialyzer({nowarn_function, yeccpars2_234_/1}).
-compile({nowarn_unused_function,  yeccpars2_234_/1}).
-file("src/absinthe_parser.yrl", 292).
yeccpars2_234_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                     build_ast_node('InputValueDefinition', #{'name' => extract_binary(___1), 'type' => ___3, 'default_value' => ___4, 'directives' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_236_/1}).
-dialyzer({nowarn_function, yeccpars2_236_/1}).
-compile({nowarn_unused_function,  yeccpars2_236_/1}).
-file("src/absinthe_parser.yrl", 277).
yeccpars2_236_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                       build_ast_node('FieldDefinition', #{'name' => extract_binary(___1), 'arguments' => ___2, 'type' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_237_/1}).
-dialyzer({nowarn_function, yeccpars2_237_/1}).
-compile({nowarn_unused_function,  yeccpars2_237_/1}).
-file("src/absinthe_parser.yrl", 279).
yeccpars2_237_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                       build_ast_node('FieldDefinition', #{'name' => extract_binary(___1), 'arguments' => ___2, 'directives' => ___5, 'type' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_239_/1}).
-dialyzer({nowarn_function, yeccpars2_239_/1}).
-compile({nowarn_unused_function,  yeccpars2_239_/1}).
-file("src/absinthe_parser.yrl", 278).
yeccpars2_239_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                   build_ast_node('FieldDefinition', #{'name' => extract_binary(___1), 'directives' => ___2, 'type' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_240_/1}).
-dialyzer({nowarn_function, yeccpars2_240_/1}).
-compile({nowarn_unused_function,  yeccpars2_240_/1}).
-file("src/absinthe_parser.yrl", 265).
yeccpars2_240_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                     ___2
  end | __Stack].

-compile({inline,yeccpars2_241_/1}).
-dialyzer({nowarn_function, yeccpars2_241_/1}).
-compile({nowarn_unused_function,  yeccpars2_241_/1}).
-file("src/absinthe_parser.yrl", 267).
yeccpars2_241_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                             [___1]
  end | __Stack].

-compile({inline,yeccpars2_242_/1}).
-dialyzer({nowarn_function, yeccpars2_242_/1}).
-compile({nowarn_unused_function,  yeccpars2_242_/1}).
-file("src/absinthe_parser.yrl", 268).
yeccpars2_242_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                           [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_245_/1}).
-dialyzer({nowarn_function, yeccpars2_245_/1}).
-compile({nowarn_unused_function,  yeccpars2_245_/1}).
-file("src/absinthe_parser.yrl", 258).
yeccpars2_245_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                 
  build_ast_node('ObjectTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3, 'fields' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_249_/1}).
-dialyzer({nowarn_function, yeccpars2_249_/1}).
-compile({nowarn_unused_function,  yeccpars2_249_/1}).
-file("src/absinthe_parser.yrl", 260).
yeccpars2_249_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                      
  build_ast_node('ObjectTypeDefinition', #{'name' => extract_binary(___2), 'interfaces' => ___3, 'fields' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_252_/1}).
-dialyzer({nowarn_function, yeccpars2_252_/1}).
-compile({nowarn_unused_function,  yeccpars2_252_/1}).
-file("src/absinthe_parser.yrl", 262).
yeccpars2_252_(__Stack0) ->
 [___7,___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                                      
  build_ast_node('ObjectTypeDefinition', #{'name' => extract_binary(___2), 'interfaces' => ___3, 'directives' => ___4, 'fields' => ___6}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_253_/1}).
-dialyzer({nowarn_function, yeccpars2_253_/1}).
-compile({nowarn_unused_function,  yeccpars2_253_/1}).
-file("src/absinthe_parser.yrl", 248).
yeccpars2_253_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                               build_ast_node('SchemaDeclaration', #{'directives' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_256_/1}).
-dialyzer({nowarn_function, yeccpars2_256_/1}).
-compile({nowarn_unused_function,  yeccpars2_256_/1}).
-file("src/absinthe_parser.yrl", 249).
yeccpars2_256_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                           build_ast_node('SchemaDeclaration', #{'fields' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_259_/1}).
-dialyzer({nowarn_function, yeccpars2_259_/1}).
-compile({nowarn_unused_function,  yeccpars2_259_/1}).
-file("src/absinthe_parser.yrl", 250).
yeccpars2_259_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                           build_ast_node('SchemaDeclaration', #{'directives' => ___2, 'fields' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_260_/1}).
-dialyzer({nowarn_function, yeccpars2_260_/1}).
-compile({nowarn_unused_function,  yeccpars2_260_/1}).
-file("src/absinthe_parser.yrl", 321).
yeccpars2_260_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                        build_ast_node('ScalarTypeDefinition', #{'name' => extract_binary(___2)}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_261_/1}).
-dialyzer({nowarn_function, yeccpars2_261_/1}).
-compile({nowarn_unused_function,  yeccpars2_261_/1}).
-file("src/absinthe_parser.yrl", 322).
yeccpars2_261_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                        build_ast_node('ScalarTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_262_/1}).
-dialyzer({nowarn_function, yeccpars2_262_/1}).
-compile({nowarn_unused_function,  yeccpars2_262_/1}).
-file("src/absinthe_parser.yrl", 294).
yeccpars2_262_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                             
  build_ast_node('InterfaceTypeDefinition', #{'name' => extract_binary(___2)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_264_/1}).
-dialyzer({nowarn_function, yeccpars2_264_/1}).
-compile({nowarn_unused_function,  yeccpars2_264_/1}).
-file("src/absinthe_parser.yrl", 296).
yeccpars2_264_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                             
  build_ast_node('InterfaceTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_267_/1}).
-dialyzer({nowarn_function, yeccpars2_267_/1}).
-compile({nowarn_unused_function,  yeccpars2_267_/1}).
-file("src/absinthe_parser.yrl", 298).
yeccpars2_267_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                         
  build_ast_node('InterfaceTypeDefinition', #{'name' => extract_binary(___2), 'fields' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_270_/1}).
-dialyzer({nowarn_function, yeccpars2_270_/1}).
-compile({nowarn_unused_function,  yeccpars2_270_/1}).
-file("src/absinthe_parser.yrl", 300).
yeccpars2_270_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                         
  build_ast_node('InterfaceTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3, 'fields' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_274_/1}).
-dialyzer({nowarn_function, yeccpars2_274_/1}).
-compile({nowarn_unused_function,  yeccpars2_274_/1}).
-file("src/absinthe_parser.yrl", 302).
yeccpars2_274_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                              
  build_ast_node('InterfaceTypeDefinition', #{'name' => extract_binary(___2), 'interfaces' => ___3, 'fields' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_277_/1}).
-dialyzer({nowarn_function, yeccpars2_277_/1}).
-compile({nowarn_unused_function,  yeccpars2_277_/1}).
-file("src/absinthe_parser.yrl", 304).
yeccpars2_277_(__Stack0) ->
 [___7,___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                                              
  build_ast_node('InterfaceTypeDefinition', #{'name' => extract_binary(___2), 'interfaces' => ___3, 'directives' => ___4, 'fields' => ___6}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_278_/1}).
-dialyzer({nowarn_function, yeccpars2_278_/1}).
-compile({nowarn_unused_function,  yeccpars2_278_/1}).
-file("src/absinthe_parser.yrl", 346).
yeccpars2_278_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                           
  build_ast_node('InputObjectTypeDefinition', #{'name' => extract_binary(___2)}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_279_/1}).
-dialyzer({nowarn_function, yeccpars2_279_/1}).
-compile({nowarn_unused_function,  yeccpars2_279_/1}).
-file("src/absinthe_parser.yrl", 348).
yeccpars2_279_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                           
  build_ast_node('InputObjectTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_282_/1}).
-dialyzer({nowarn_function, yeccpars2_282_/1}).
-compile({nowarn_unused_function,  yeccpars2_282_/1}).
-file("src/absinthe_parser.yrl", 350).
yeccpars2_282_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                            
  build_ast_node('InputObjectTypeDefinition', #{'name' => extract_binary(___2), 'fields' => ___4}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_285_/1}).
-dialyzer({nowarn_function, yeccpars2_285_/1}).
-compile({nowarn_unused_function,  yeccpars2_285_/1}).
-file("src/absinthe_parser.yrl", 352).
yeccpars2_285_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                            
  build_ast_node('InputObjectTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3, 'fields' => ___5}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_289_/1}).
-dialyzer({nowarn_function, yeccpars2_289_/1}).
-compile({nowarn_unused_function,  yeccpars2_289_/1}).
-file("src/absinthe_parser.yrl", 49).
yeccpars2_289_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                      build_ast_node('Fragment', #{'name' => ___2, 'type_condition' => ___4, 'selection_set' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_291_/1}).
-dialyzer({nowarn_function, yeccpars2_291_/1}).
-compile({nowarn_unused_function,  yeccpars2_291_/1}).
-file("src/absinthe_parser.yrl", 50).
yeccpars2_291_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                 build_ast_node('Fragment', #{'name' => ___2, 'type_condition' => ___4, 'directives' => ___5, 'selection_set' => ___6}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_292_/1}).
-dialyzer({nowarn_function, yeccpars2_292_/1}).
-compile({nowarn_unused_function,  yeccpars2_292_/1}).
-file("src/absinthe_parser.yrl", 367).
yeccpars2_292_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                         
  build_ast_node('TypeExtensionDefinition', #{'definition' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_293_/1}).
-dialyzer({nowarn_function, yeccpars2_293_/1}).
-compile({nowarn_unused_function,  yeccpars2_293_/1}).
-file("src/absinthe_parser.yrl", 365).
yeccpars2_293_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                      
  build_ast_node('TypeExtensionDefinition', #{'definition' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_294_/1}).
-dialyzer({nowarn_function, yeccpars2_294_/1}).
-compile({nowarn_unused_function,  yeccpars2_294_/1}).
-file("src/absinthe_parser.yrl", 363).
yeccpars2_294_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                          
  build_ast_node('TypeExtensionDefinition', #{'definition' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_295_/1}).
-dialyzer({nowarn_function, yeccpars2_295_/1}).
-compile({nowarn_unused_function,  yeccpars2_295_/1}).
-file("src/absinthe_parser.yrl", 361).
yeccpars2_295_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                          
  build_ast_node('TypeExtensionDefinition', #{'definition' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_296_/1}).
-dialyzer({nowarn_function, yeccpars2_296_/1}).
-compile({nowarn_unused_function,  yeccpars2_296_/1}).
-file("src/absinthe_parser.yrl", 359).
yeccpars2_296_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                             
  build_ast_node('TypeExtensionDefinition', #{'definition' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_297_/1}).
-dialyzer({nowarn_function, yeccpars2_297_/1}).
-compile({nowarn_unused_function,  yeccpars2_297_/1}).
-file("src/absinthe_parser.yrl", 357).
yeccpars2_297_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                               
  build_ast_node('TypeExtensionDefinition', #{'definition' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_298_/1}).
-dialyzer({nowarn_function, yeccpars2_298_/1}).
-compile({nowarn_unused_function,  yeccpars2_298_/1}).
-file("src/absinthe_parser.yrl", 355).
yeccpars2_298_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                        
  build_ast_node('TypeExtensionDefinition', #{'definition' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_299_/1}).
-dialyzer({nowarn_function, yeccpars2_299_/1}).
-compile({nowarn_unused_function,  yeccpars2_299_/1}).
-file("src/absinthe_parser.yrl", 324).
yeccpars2_299_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                   
  build_ast_node('EnumTypeDefinition', #{'name' => extract_binary(___2)}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_300_/1}).
-dialyzer({nowarn_function, yeccpars2_300_/1}).
-compile({nowarn_unused_function,  yeccpars2_300_/1}).
-file("src/absinthe_parser.yrl", 326).
yeccpars2_300_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                   
  build_ast_node('EnumTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_303_/1}).
-dialyzer({nowarn_function, yeccpars2_303_/1}).
-compile({nowarn_unused_function,  yeccpars2_303_/1}).
-file("src/absinthe_parser.yrl", 333).
yeccpars2_303_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                                 [___1]
  end | __Stack].

-compile({inline,yeccpars2_304_/1}).
-dialyzer({nowarn_function, yeccpars2_304_/1}).
-compile({nowarn_unused_function,  yeccpars2_304_/1}).
-file("src/absinthe_parser.yrl", 343).
yeccpars2_304_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                   build_ast_node('EnumValueDefinition', #{'value' => extract_binary(___1)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_306_/1}).
-dialyzer({nowarn_function, yeccpars2_306_/1}).
-compile({nowarn_unused_function,  yeccpars2_306_/1}).
-file("src/absinthe_parser.yrl", 336).
yeccpars2_306_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                                       [put_description(___2, ___1)]
  end | __Stack].

-compile({inline,yeccpars2_307_/1}).
-dialyzer({nowarn_function, yeccpars2_307_/1}).
-compile({nowarn_unused_function,  yeccpars2_307_/1}).
-file("src/absinthe_parser.yrl", 337).
yeccpars2_307_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                               [put_description(___2, ___1)|___3]
  end | __Stack].

-compile({inline,yeccpars2_308_/1}).
-dialyzer({nowarn_function, yeccpars2_308_/1}).
-compile({nowarn_unused_function,  yeccpars2_308_/1}).
-file("src/absinthe_parser.yrl", 344).
yeccpars2_308_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                   build_ast_node('EnumValueDefinition', #{'value' => extract_binary(___1), 'directives' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_309_/1}).
-dialyzer({nowarn_function, yeccpars2_309_/1}).
-compile({nowarn_unused_function,  yeccpars2_309_/1}).
-file("src/absinthe_parser.yrl", 334).
yeccpars2_309_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                                         [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_310_/1}).
-dialyzer({nowarn_function, yeccpars2_310_/1}).
-compile({nowarn_unused_function,  yeccpars2_310_/1}).
-file("src/absinthe_parser.yrl", 328).
yeccpars2_310_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                  
  build_ast_node('EnumTypeDefinition', #{'name' => extract_binary(___2), 'values' => ___4}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_313_/1}).
-dialyzer({nowarn_function, yeccpars2_313_/1}).
-compile({nowarn_unused_function,  yeccpars2_313_/1}).
-file("src/absinthe_parser.yrl", 330).
yeccpars2_313_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                  
  build_ast_node('EnumTypeDefinition', #{'name' => extract_binary(___2), 'directives' => ___3, 'values' => ___5}, extract_location(___2))
  end | __Stack].

-compile({inline,yeccpars2_320_/1}).
-dialyzer({nowarn_function, yeccpars2_320_/1}).
-compile({nowarn_unused_function,  yeccpars2_320_/1}).
-file("src/absinthe_parser.yrl", 339).
yeccpars2_320_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                       [extract_binary(___1)]
  end | __Stack].

-compile({inline,yeccpars2_321_/1}).
-dialyzer({nowarn_function, yeccpars2_321_/1}).
-compile({nowarn_unused_function,  yeccpars2_321_/1}).
-file("src/absinthe_parser.yrl", 235).
yeccpars2_321_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                            
  build_ast_node('DirectiveDefinition', #{'name' => extract_binary(___3), 'locations' => extract_directive_locations(___6), 'repeatable' => true}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_325_/1}).
-dialyzer({nowarn_function, yeccpars2_325_/1}).
-compile({nowarn_unused_function,  yeccpars2_325_/1}).
-file("src/absinthe_parser.yrl", 341).
yeccpars2_325_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                            [extract_binary(___2)|___4]
  end | __Stack].

-compile({inline,yeccpars2_326_/1}).
-dialyzer({nowarn_function, yeccpars2_326_/1}).
-compile({nowarn_unused_function,  yeccpars2_326_/1}).
-file("src/absinthe_parser.yrl", 240).
yeccpars2_326_(__Stack0) ->
 [___7,___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                                       
  build_ast_node('DirectiveDefinition', #{'name' => extract_binary(___3), 'directives' => ___7, 'locations' => extract_directive_locations(___6), 'repeatable' => true}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_328_/1}).
-dialyzer({nowarn_function, yeccpars2_328_/1}).
-compile({nowarn_unused_function,  yeccpars2_328_/1}).
-file("src/absinthe_parser.yrl", 340).
yeccpars2_328_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                        [extract_binary(___1)|___3]
  end | __Stack].

-compile({inline,yeccpars2_329_/1}).
-dialyzer({nowarn_function, yeccpars2_329_/1}).
-compile({nowarn_unused_function,  yeccpars2_329_/1}).
-file("src/absinthe_parser.yrl", 225).
yeccpars2_329_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                               
  build_ast_node('DirectiveDefinition', #{'name' => extract_binary(___3), 'locations' => extract_directive_locations(___5)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_330_/1}).
-dialyzer({nowarn_function, yeccpars2_330_/1}).
-compile({nowarn_unused_function,  yeccpars2_330_/1}).
-file("src/absinthe_parser.yrl", 230).
yeccpars2_330_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                          
  build_ast_node('DirectiveDefinition', #{'name' => extract_binary(___3), 'directives' => ___6, 'locations' => extract_directive_locations(___5)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_334_/1}).
-dialyzer({nowarn_function, yeccpars2_334_/1}).
-compile({nowarn_unused_function,  yeccpars2_334_/1}).
-file("src/absinthe_parser.yrl", 237).
yeccpars2_334_(__Stack0) ->
 [___7,___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                                                
  build_ast_node('DirectiveDefinition', #{'name' => extract_binary(___3), 'arguments' => ___4, 'locations' => extract_directive_locations(___7), 'repeatable' => true}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_335_/1}).
-dialyzer({nowarn_function, yeccpars2_335_/1}).
-compile({nowarn_unused_function,  yeccpars2_335_/1}).
-file("src/absinthe_parser.yrl", 242).
yeccpars2_335_(__Stack0) ->
 [___8,___7,___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                                                           
  build_ast_node('DirectiveDefinition', #{'name' => extract_binary(___3), 'arguments' => ___4, 'directives' => ___8, 'locations' => extract_directive_locations(___7), 'repeatable' => true}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_336_/1}).
-dialyzer({nowarn_function, yeccpars2_336_/1}).
-compile({nowarn_unused_function,  yeccpars2_336_/1}).
-file("src/absinthe_parser.yrl", 227).
yeccpars2_336_(__Stack0) ->
 [___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                                   
  build_ast_node('DirectiveDefinition', #{'name' => extract_binary(___3), 'arguments' => ___4, 'locations' => extract_directive_locations(___6)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_337_/1}).
-dialyzer({nowarn_function, yeccpars2_337_/1}).
-compile({nowarn_unused_function,  yeccpars2_337_/1}).
-file("src/absinthe_parser.yrl", 232).
yeccpars2_337_(__Stack0) ->
 [___7,___6,___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                                              
  build_ast_node('DirectiveDefinition', #{'name' => extract_binary(___3), 'arguments' => ___4, 'directives' => ___7, 'locations' => extract_directive_locations(___6)}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_338_/1}).
-dialyzer({nowarn_function, yeccpars2_338_/1}).
-compile({nowarn_unused_function,  yeccpars2_338_/1}).
-file("src/absinthe_parser.yrl", 27).
yeccpars2_338_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                        [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_339_/1}).
-dialyzer({nowarn_function, yeccpars2_339_/1}).
-compile({nowarn_unused_function,  yeccpars2_339_/1}).
-file("src/absinthe_parser.yrl", 219).
yeccpars2_339_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                              put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_340_/1}).
-dialyzer({nowarn_function, yeccpars2_340_/1}).
-compile({nowarn_unused_function,  yeccpars2_340_/1}).
-file("src/absinthe_parser.yrl", 216).
yeccpars2_340_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                           put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_341_/1}).
-dialyzer({nowarn_function, yeccpars2_341_/1}).
-compile({nowarn_unused_function,  yeccpars2_341_/1}).
-file("src/absinthe_parser.yrl", 220).
yeccpars2_341_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                               put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_342_/1}).
-dialyzer({nowarn_function, yeccpars2_342_/1}).
-compile({nowarn_unused_function,  yeccpars2_342_/1}).
-file("src/absinthe_parser.yrl", 30).
yeccpars2_342_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                          put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_343_/1}).
-dialyzer({nowarn_function, yeccpars2_343_/1}).
-compile({nowarn_unused_function,  yeccpars2_343_/1}).
-file("src/absinthe_parser.yrl", 217).
yeccpars2_343_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                               put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_344_/1}).
-dialyzer({nowarn_function, yeccpars2_344_/1}).
-compile({nowarn_unused_function,  yeccpars2_344_/1}).
-file("src/absinthe_parser.yrl", 218).
yeccpars2_344_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                                  put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_345_/1}).
-dialyzer({nowarn_function, yeccpars2_345_/1}).
-compile({nowarn_unused_function,  yeccpars2_345_/1}).
-file("src/absinthe_parser.yrl", 222).
yeccpars2_345_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                                    put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_346_/1}).
-dialyzer({nowarn_function, yeccpars2_346_/1}).
-compile({nowarn_unused_function,  yeccpars2_346_/1}).
-file("src/absinthe_parser.yrl", 32).
yeccpars2_346_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                               put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_347_/1}).
-dialyzer({nowarn_function, yeccpars2_347_/1}).
-compile({nowarn_unused_function,  yeccpars2_347_/1}).
-file("src/absinthe_parser.yrl", 221).
yeccpars2_347_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                             put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_348_/1}).
-dialyzer({nowarn_function, yeccpars2_348_/1}).
-compile({nowarn_unused_function,  yeccpars2_348_/1}).
-file("src/absinthe_parser.yrl", 223).
yeccpars2_348_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                              put_description(___2, ___1)
  end | __Stack].

-compile({inline,yeccpars2_350_/1}).
-dialyzer({nowarn_function, yeccpars2_350_/1}).
-compile({nowarn_unused_function,  yeccpars2_350_/1}).
-file("src/absinthe_parser.yrl", 40).
yeccpars2_350_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                    build_ast_node('OperationDefinition', #{'operation' => extract_atom(___1), 'selection_set' => ___2}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_355_/1}).
-dialyzer({nowarn_function, yeccpars2_355_/1}).
-compile({nowarn_unused_function,  yeccpars2_355_/1}).
-file("src/absinthe_parser.yrl", 55).
yeccpars2_355_(__Stack0) ->
 [___1 | __Stack] = __Stack0,
 [begin
                                               [___1]
  end | __Stack].

-compile({inline,yeccpars2_358_/1}).
-dialyzer({nowarn_function, yeccpars2_358_/1}).
-compile({nowarn_unused_function,  yeccpars2_358_/1}).
-file("src/absinthe_parser.yrl", 57).
yeccpars2_358_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                          build_ast_node('VariableDefinition', #{'variable' => ___1, 'type' => ___3}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_359_/1}).
-dialyzer({nowarn_function, yeccpars2_359_/1}).
-compile({nowarn_unused_function,  yeccpars2_359_/1}).
-file("src/absinthe_parser.yrl", 58).
yeccpars2_359_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                         build_ast_node('VariableDefinition', #{'variable' => ___1, 'type' => ___3}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_360_/1}).
-dialyzer({nowarn_function, yeccpars2_360_/1}).
-compile({nowarn_unused_function,  yeccpars2_360_/1}).
-file("src/absinthe_parser.yrl", 59).
yeccpars2_360_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                       build_ast_node('VariableDefinition', #{'variable' => ___1, 'type' => ___3, 'default_value' => ___4}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_361_/1}).
-dialyzer({nowarn_function, yeccpars2_361_/1}).
-compile({nowarn_unused_function,  yeccpars2_361_/1}).
-file("src/absinthe_parser.yrl", 60).
yeccpars2_361_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                       build_ast_node('VariableDefinition', #{'variable' => ___1, 'type' => ___3, 'default_value' => ___4, 'directives' => ___5}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_362_/1}).
-dialyzer({nowarn_function, yeccpars2_362_/1}).
-compile({nowarn_unused_function,  yeccpars2_362_/1}).
-file("src/absinthe_parser.yrl", 56).
yeccpars2_362_(__Stack0) ->
 [___2,___1 | __Stack] = __Stack0,
 [begin
                                                                      [___1|___2]
  end | __Stack].

-compile({inline,yeccpars2_363_/1}).
-dialyzer({nowarn_function, yeccpars2_363_/1}).
-compile({nowarn_unused_function,  yeccpars2_363_/1}).
-file("src/absinthe_parser.yrl", 54).
yeccpars2_363_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                        ___2
  end | __Stack].

-compile({inline,yeccpars2_364_/1}).
-dialyzer({nowarn_function, yeccpars2_364_/1}).
-compile({nowarn_unused_function,  yeccpars2_364_/1}).
-file("src/absinthe_parser.yrl", 41).
yeccpars2_364_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                               build_ast_node('OperationDefinition', #{'operation' => extract_atom(___1), 'directives' => ___2, 'selection_set' => ___3}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_366_/1}).
-dialyzer({nowarn_function, yeccpars2_366_/1}).
-compile({nowarn_unused_function,  yeccpars2_366_/1}).
-file("src/absinthe_parser.yrl", 44).
yeccpars2_366_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                         build_ast_node('OperationDefinition', #{'operation' => extract_atom(___1), 'name' => extract_binary(___2), 'selection_set' => ___3}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_368_/1}).
-dialyzer({nowarn_function, yeccpars2_368_/1}).
-compile({nowarn_unused_function,  yeccpars2_368_/1}).
-file("src/absinthe_parser.yrl", 46).
yeccpars2_368_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                    build_ast_node('OperationDefinition', #{'operation' => extract_atom(___1), 'name' => extract_binary(___2), 'directives' => ___3, 'selection_set' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_369_/1}).
-dialyzer({nowarn_function, yeccpars2_369_/1}).
-compile({nowarn_unused_function,  yeccpars2_369_/1}).
-file("src/absinthe_parser.yrl", 45).
yeccpars2_369_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                             build_ast_node('OperationDefinition', #{'operation' => extract_atom(___1), 'name' => extract_binary(___2), 'variable_definitions' => ___3, 'selection_set' => ___4}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_371_/1}).
-dialyzer({nowarn_function, yeccpars2_371_/1}).
-compile({nowarn_unused_function,  yeccpars2_371_/1}).
-file("src/absinthe_parser.yrl", 47).
yeccpars2_371_(__Stack0) ->
 [___5,___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                        build_ast_node('OperationDefinition', #{'operation' => extract_atom(___1), 'name' => extract_binary(___2), 'variable_definitions' => ___3, 'directives' => ___4, 'selection_set' => ___5}, extract_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_372_/1}).
-dialyzer({nowarn_function, yeccpars2_372_/1}).
-compile({nowarn_unused_function,  yeccpars2_372_/1}).
-file("src/absinthe_parser.yrl", 42).
yeccpars2_372_(__Stack0) ->
 [___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                        build_ast_node('OperationDefinition', #{'operation' => extract_atom(___1), 'variable_definitions' => ___2, 'selection_set' => ___3}, extract_child_location(___1))
  end | __Stack].

-compile({inline,yeccpars2_374_/1}).
-dialyzer({nowarn_function, yeccpars2_374_/1}).
-compile({nowarn_unused_function,  yeccpars2_374_/1}).
-file("src/absinthe_parser.yrl", 43).
yeccpars2_374_(__Stack0) ->
 [___4,___3,___2,___1 | __Stack] = __Stack0,
 [begin
                                                                                   build_ast_node('OperationDefinition', #{'operation' => extract_atom(___1), 'variable_definitions' => ___2, 'directives' => ___3, 'selection_set' => ___4}, extract_child_location(___1))
  end | __Stack].


-file("src/absinthe_parser.yrl", 556).
