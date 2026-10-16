// CC0. Compiles a ReShade effect with ReShade's own FX compiler, as ReShade does when it loads
// one, and fails on any error: the ReShade port's check, since nothing else here runs ReShade.
//
//   check [-I DIR]... [-D NAME=VALUE]... [-o OUT.spv] EFFECT.fx
//
// Built against ReShade's compiler sources (effect_*.cpp); see build.sh. Warnings are printed
// and fail the check too, so the port compiles cleanly.

#include "effect_codegen.hpp"
#include "effect_parser.hpp"
#include "effect_preprocessor.hpp"
#include <cstdio>
#include <fstream>
#include <iostream>
#include <memory>
#include <string>

int main(int argc, char **argv)
{
	reshadefx::preprocessor preprocessor;
	std::string effect, out;
	for (int i = 1; i < argc; i++) {
		std::string arg = argv[i];
		if (arg == "-I" && i + 1 < argc) {
			preprocessor.add_include_path(argv[++i]);
		} else if (arg == "-D" && i + 1 < argc) {
			std::string definition = argv[++i];
			size_t equals = definition.find('=');
			if (equals == std::string::npos)
				preprocessor.add_macro_definition(definition);
			else
				preprocessor.add_macro_definition(definition.substr(0, equals),
								  definition.substr(equals + 1));
		} else if (arg == "-o" && i + 1 < argc) {
			out = argv[++i];
		} else {
			effect = arg;
		}
	}
	if (effect.empty()) {
		std::cerr << "usage: check [-I DIR]... [-D NAME=VALUE]... [-o OUT.spv] EFFECT.fx\n";
		return 2;
	}
	if (!preprocessor.append_file(effect)) {
		std::cerr << preprocessor.errors() << "cannot preprocess " << effect << "\n";
		return 1;
	}
	std::string messages = preprocessor.errors();
	reshadefx::parser parser;
	std::unique_ptr<reshadefx::codegen> codegen(
		reshadefx::create_codegen_spirv(true, false, false, true));
	bool parsed = parser.parse(preprocessor.output(), codegen.get());
	messages += parser.errors();
	reshadefx::module module;
	codegen->write_result(module);
	if (!messages.empty())
		std::cerr << messages;
	if (!parsed || !messages.empty())
		return 1;
	for (const auto &technique : module.techniques) {
		std::cout << "technique " << technique.name << ":";
		for (const auto &pass : technique.passes)
			std::cout << " " << pass.ps_entry_point;
		std::cout << "\n";
	}
	std::cout << module.textures.size() << " textures, " << module.samplers.size()
		  << " samplers, " << module.uniforms.size() << " uniforms\n";
	if (!out.empty()) {
		std::ofstream file(out, std::ios::binary);
		file.write(reinterpret_cast<const char *>(module.spirv.data()),
			   module.spirv.size() * sizeof(uint32_t));
	}
	return 0;
}
